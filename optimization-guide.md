# VibeMQ Optimization Guide: Lessons from FlashMQ

A deep analysis of why FlashMQ outperforms VibeMQ on QoS 2 fan-out by 3x latency and 10x memory, with specific recommendations for improvement.

## Progress

Summary Table

| Optimization          | Effort    | Impact | Already Have?    |
|-----------------------|-----------|--------|------------------|
| Thread-local HashMap  | 1 day     | Low    | ✅ Done          |
| Arc for topic         | 1 day     | Medium | ✅ Done          |
| Pre-serialize packets | 1 week    | High   | ✅ Done          |
| Batch channel sends   | 3-5 days  | Medium | No               |
| Direct write path     | 2-4 weeks | High   | No - big rewrite |
| Per-node trie locks   | 1-2 weeks | Low    | No               |

### Implementation Notes

**Pre-serialize packets:** Implemented in `src/codec/cached.rs`. Uses `CachedPublish` struct with pre-serialized bytes and offset patching for first_byte (dup/qos/retain flags) and packet_id (2 bytes). Works for all QoS levels:

- **QoS 0:** Uses `CachedPublish::new_qos0()` without packet_id space
- **QoS 1/2:** Uses `CachedPublish::new()` with packet_id at known offset

For messages without subscription identifiers, fan-out uses memcpy + patch instead of per-subscriber encoding. The `InflightMessage` enum supports both Cached (pre-serialized) and Full (original Publish) variants:

- **Cached:** For efficient retransmission, stores Arc<CachedPublish>
- **Full:** For messages with subscription IDs in properties (fallback path)

Retry and session resume use `cached.write_to()` with dup=true for efficient resend.

## Benchmark Context

**Scenario:** QoS 2 Fan-Out (100 publishers → 2000 subscribers, ~190K msg/s)

| Metric | FlashMQ (C++) | VibeMQ (Rust) | Gap |
|--------|---------------|---------------|-----|
| Mean Latency | 80ms | 251ms | 3.2x |
| P99 Latency | 165ms | 552ms | 3.3x |
| CPU Usage | 64% | 215% | 3.3x |
| Memory | 41 MB | 394 MB | 9.6x |

Both achieved 100% delivery at identical throughput. The difference is pure efficiency.

---

## Executive Summary: Why FlashMQ Wins

| Optimization | FlashMQ | VibeMQ | Impact |
|--------------|---------|--------|--------|
| Packet serialization | Once per publish | Once per subscriber | **N× fewer serializations** |
| Fan-out allocations | 3-4 total | N+2 (N = subscribers) | **~2000× fewer allocations** |
| Message cloning | In-place packet ID modification | Full Publish struct clone | **Zero-copy vs O(payload) copy** |
| Dedup structure | Pre-sized vector, reused | New HashMap per publish | **No per-publish allocation** |
| Async overhead | Direct epoll + memcpy | Tokio channels + task scheduling | **No runtime overhead** |
| Lock granularity | Per-trie-node shared_mutex | Global RwLock on trie | **Better parallelism** |

---

## Part 1: FlashMQ Hot Path Analysis

### 1.1 PublishCopyFactory — The Key Innovation

**File:** `FlashMQ/publishcopyfactory.cpp:33-82`

FlashMQ pre-serializes each publish packet **once**, then reuses it for all subscribers:

```cpp
// Cache key: protocol version + QoS level (only 4 possible combinations)
const int layout_key_target = getPublishLayoutCompareKey(protocolVersion, actualQos);

// Fast path: reuse original packet if layout matches
if (!packet->biteArrayCannotBeReused() &&
    getPublishLayoutCompareKey(packet->getProtocolVersion(), orgQos) == layout_key_target) {
    return packet;  // ZERO-COPY: return pointer to existing packet
}

// Slow path: create cached variant (max 4 per publish)
std::optional<MqttPacket> &cachedPack = constructedPacketCache[layout_key_target];
if (!cachedPack) {
    cachedPack.emplace(protocolVersion, packet->getPublishData(), actualQos, ...);
}
return &*cachedPack;
```

**Result:** For 2000 subscribers, FlashMQ creates **1-4 packet buffers** (one per protocol/QoS combination), not 2000.

### 1.2 In-Place Packet ID Modification

**File:** `FlashMQ/client.cpp:414-422`

Each subscriber needs a unique packet ID, but FlashMQ doesn't re-serialize:

```cpp
if (p->getQos() > 0) {
    p->setPacketId(packet_id);  // Writes 2 bytes at pre-calculated offset
    p->setQos(copyFactory.getEffectiveQos(max_qos));  // Modifies first_byte
}
p->setRetain(retain);  // Modifies first_byte
```

The packet structure stores `packet_id_pos` during initial serialization, allowing O(1) modification without re-encoding.

### 1.3 Pre-Sized Subscriber Vector

**File:** `FlashMQ/subscriptionstore.cpp:661-672`

```cpp
const size_t reserve = this->subscriber_reserve.load(std::memory_order_relaxed);
std::vector<ReceivingSubscriber> subscriberSessions;
subscriberSessions.reserve(reserve);  // Pre-allocate based on historical peak

// Adaptive sizing: remember high-water mark
if (subscriberSessions.size() > reserve && subscriberSessions.size() <= 1048576)
    this->subscriber_reserve.store(reserve, std::memory_order_relaxed);
```

No per-publish allocation — the vector grows once and stays sized for peak load.

### 1.4 Optimistic Weak Pointer Insertion

**File:** `FlashMQ/subscriptionstore.cpp:513-535`

```cpp
for (auto &pair : this_node->subscribers) {
    const Subscription &sub = pair.second;

    // Insert first, check validity after (avoids temporary shared_ptr)
    targetSessions.emplace_back(sub.session, sub.qos, ...);

    if (!targetSessions.back().session) {
        targetSessions.pop_back();  // Remove if session expired
        continue;
    }
}
```

Comment from source: *"By not using a temporary locked shared_ptr<Session> for checks, doing an optimistic insertion instead, we avoid unnecessary copies."*

### 1.5 Direct Buffer Writes

**File:** `FlashMQ/client.cpp:352`, `FlashMQ/mqttpacket.cpp:2699`

```cpp
// Single memcpy of pre-serialized packet into circular buffer
packet.readIntoBuf(write_buf_locked->buf);

// Inside readIntoBuf:
buf.writerange(bites.begin(), bites.end());  // std::copy → memcpy
```

No channel send, no task wake-up, no async overhead — just memcpy into the write buffer.

### 1.6 Allocation Summary: FlashMQ

For a publish to 2000 subscribers at QoS 2:

| Component | Allocations |
|-----------|-------------|
| PublishCopyFactory cache | 1-4 (protocol × QoS variants) |
| Subscriber vector | 0 (pre-sized) |
| Per-subscriber | 0 (in-place modification) |
| QoS queue (offline only) | 0-N (only offline clients) |
| **Total** | **1-4** |

---

## Part 2: VibeMQ Hot Path Analysis

### 2.1 Per-Publish HashMap Allocation

**File:** `vibemq/src/broker/connection/publish.rs:274`

```rust
let mut client_subs: AHashMap<Arc<str>, ClientSub> =
    AHashMap::with_capacity(matches.len());
```

Every single publish allocates a new HashMap for deduplication. With 100 msg/s × 60s = 6000 publishes, that's **6000 HashMap allocations** in the benchmark.

### 2.2 Message Clone Per Subscriber

**File:** `vibemq/src/broker/connection/publish.rs:311`

```rust
for (client_id, sub_info) in client_subs {
    let mut outgoing = publish.clone();  // FULL CLONE

    // ... modify fields ...

    if let Err(_) = sender.try_send(Packet::Publish(outgoing)) {
        // handle backpressure
    }
}
```

Each of 2000 subscribers gets a **full clone** of the Publish struct:

```rust
pub struct Publish {
    pub dup: bool,
    pub qos: QoS,
    pub retain: bool,
    pub topic: TopicName,          // String clone
    pub packet_id: Option<u16>,
    pub properties: PublishProperties,  // May contain user properties
    pub payload: Bytes,            // Arc increment (cheap)
}
```

While `Bytes` is reference-counted (cheap), the struct itself, topic string, and properties are all cloned.

### 2.3 Channel Send Overhead

**File:** `vibemq/src/broker/connection/publish.rs:329`

```rust
sender.try_send(Packet::Publish(outgoing))
```

Each `try_send`:
1. Acquires channel lock
2. Pushes to VecDeque
3. Wakes receiver task (if waiting)
4. Returns

With 2000 subscribers, that's 2000 channel operations per publish.

### 2.4 Tokio Task Wake-ups

Each channel send potentially wakes the receiver task:

```rust
// In connection event loop (src/broker/connection/mod.rs:235-238)
Some(packet) = self.packet_rx.recv() => {
    self.handle_outgoing_packet(&session, packet).await?;
}
```

2000 task wake-ups per publish, each involving:
- Scheduler queue insertion
- Potential context switch
- Cache line invalidation

### 2.5 QoS 2 Double Clone

**File:** `vibemq/src/broker/connection/publish.rs:201` and `mod.rs:324`

```rust
// Incoming QoS 2: clone for storage
s.inflight_incoming.insert(packet_id, publish.clone());

// Outgoing QoS 2: clone for retry tracking
s.inflight_outgoing.insert(packet_id, InflightMessage {
    publish: publish.clone(),
    qos2_state: Some(Qos2State::WaitingPubRec),
    ...
});
```

QoS 2 messages are cloned **twice** beyond the fan-out clones.

### 2.6 Allocation Summary: VibeMQ

For a publish to 2000 subscribers at QoS 2:

| Component | Allocations |
|-----------|-------------|
| Dedup HashMap | 1 |
| Per-subscriber clone | 2000 |
| Channel queue nodes | 2000 |
| QoS 2 storage | 2000 |
| **Total** | **~6000** |

---

## Part 3: Specific Recommendations

### 3.1 Pre-Serialize Packets (HIGH IMPACT)

**Current:** Publish struct cloned per subscriber, serialized on write
**Target:** Serialize once, memcpy to each subscriber

```rust
// New structure
pub struct SerializedPublish {
    // Pre-encoded packet bytes (without packet ID)
    bytes: Bytes,
    // Offset where packet ID should be written
    packet_id_offset: Option<usize>,
    // Metadata for QoS/retain modification
    first_byte_offset: usize,
}

impl SerializedPublish {
    /// Create from Publish, encoding once
    pub fn new(publish: &Publish, protocol: ProtocolVersion) -> Self {
        let mut buf = BytesMut::with_capacity(publish.encoded_size());
        publish.encode(&mut buf, protocol);
        // ... store offsets for mutable fields
    }

    /// Write to buffer with specific packet ID (no re-encoding)
    pub fn write_with_id(&self, buf: &mut BytesMut, packet_id: u16) {
        buf.extend_from_slice(&self.bytes);
        if let Some(offset) = self.packet_id_offset {
            buf[offset..offset+2].copy_from_slice(&packet_id.to_be_bytes());
        }
    }
}
```

**Expected impact:** Eliminate 2000 serializations per publish → ~50% latency reduction

### 3.2 Reuse Dedup HashMap (MEDIUM IMPACT)

**Current:** New HashMap per publish
**Target:** Thread-local or pooled HashMap

```rust
thread_local! {
    static DEDUP_MAP: RefCell<AHashMap<Arc<str>, ClientSub>> =
        RefCell::new(AHashMap::with_capacity(64));
}

fn route_message(&self, publish: &Publish) {
    DEDUP_MAP.with(|map| {
        let mut map = map.borrow_mut();
        map.clear();  // Reuse allocation

        // ... fill map ...

        for (client_id, sub_info) in map.drain() {
            // ... fan out ...
        }
    });
}
```

**Expected impact:** Eliminate 6000 HashMap allocations per minute → reduced GC pressure

### 3.3 Arc-Wrap Publish for Fan-Out (HIGH IMPACT)

**Current:** Clone Publish struct per subscriber
**Target:** Share single Arc<Publish>, clone only Arc pointer

```rust
pub struct FanOutMessage {
    inner: Arc<PublishInner>,
    packet_id: u16,      // Per-subscriber
    retain: bool,        // Per-subscriber (can differ by subscription)
}

struct PublishInner {
    topic: TopicName,
    payload: Bytes,
    properties: PublishProperties,
    qos: QoS,
}

impl Clone for FanOutMessage {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),  // Just pointer copy
            packet_id: self.packet_id,
            retain: self.retain,
        }
    }
}
```

**Expected impact:** Eliminate 2000 struct clones per publish → ~30% memory reduction

### 3.4 Batch Channel Sends (MEDIUM IMPACT)

**Current:** One channel send per subscriber
**Target:** Batch messages, reduce wake-ups

```rust
// Collect messages for same connection
let mut batches: AHashMap<Arc<str>, SmallVec<[Packet; 8]>> = AHashMap::new();

for (client_id, sub_info) in client_subs {
    batches.entry(client_id)
        .or_default()
        .push(Packet::Publish(outgoing));
}

// Single send per connection
for (client_id, packets) in batches {
    if let Some(sender) = self.connections.get(&client_id) {
        sender.try_send(Packet::Batch(packets));
    }
}
```

**Expected impact:** Reduce channel operations by batching factor → lower lock contention

### 3.5 Direct Write Path (HIGH IMPACT, COMPLEX)

**Current:** Channel → Task → Write
**Target:** Direct write for online clients (like FlashMQ)

```rust
impl Connection {
    /// Try direct write, fall back to channel if would block
    fn try_direct_write(&self, packet: &SerializedPublish) -> Result<(), Packet> {
        let mut buf = self.write_buf.try_lock().ok()?;

        if buf.remaining_capacity() >= packet.len() {
            packet.write_to(&mut buf);
            Ok(())
        } else {
            Err(packet.to_queued())  // Fall back to channel
        }
    }
}
```

This is a significant architectural change but eliminates async overhead for the fast path.

**Expected impact:** Eliminate 2000 task wake-ups per publish → ~40% latency reduction

### 3.6 Per-Node Trie Locks (LOW IMPACT for fan-out)

**Current:** Single RwLock on entire trie
**Target:** Per-node locks like FlashMQ

```rust
struct TrieNode<V> {
    lock: parking_lot::RwLock<TrieNodeInner<V>>,
}

struct TrieNodeInner<V> {
    children: AHashMap<CompactString, Arc<TrieNode<V>>>,
    single_wildcard: Option<Arc<TrieNode<V>>>,
    multi_wildcard: Option<V>,
    subscribers: Vec<V>,
}
```

**Expected impact:** Better parallelism for concurrent publishes to different topics

---

## Part 4: Implementation Priority

### Phase 1: Quick Wins (1-2 days each)

1. **Reuse dedup HashMap** — Thread-local, minimal code change
2. **Arc-wrap Publish** — Reduces clone cost significantly
3. **Pre-size subscriber vector** — Adaptive like FlashMQ

### Phase 2: Medium Effort (1 week each)

4. **Pre-serialize packets** — Requires encoder refactoring
5. **Batch channel sends** — Requires protocol change for batched packets

### Phase 3: Architectural (2-4 weeks)

6. **Direct write path** — Major change to connection model
7. **Per-node trie locks** — Requires careful lock ordering

---

## Part 5: Expected Results

If all optimizations implemented:

| Metric | Current | Target | Improvement |
|--------|---------|--------|-------------|
| Mean Latency | 251ms | ~80-100ms | 2.5-3x |
| P99 Latency | 552ms | ~150-200ms | 2.5-3x |
| CPU Usage | 215% | ~80-100% | 2x |
| Memory | 394 MB | ~60-80 MB | 5-6x |

FlashMQ's numbers (80ms, 64%, 41MB) represent the theoretical optimum for this workload. With Rust's zero-cost abstractions, VibeMQ should be able to approach these numbers while maintaining memory safety.

---

## Part 6: Safety Considerations

A key advantage of VibeMQ is Rust's memory safety. Not all FlashMQ optimizations can be directly ported without trade-offs.

### Optimizations That Are 100% Safe

| Optimization | Why Safe | Rust Idiom |
|--------------|----------|------------|
| Arc-wrap Publish | Compiler-enforced sharing | `Arc<T>` |
| Thread-local HashMap | No cross-thread sharing | `thread_local!` |
| Pre-size vectors | Just capacity hints | `Vec::with_capacity()` |
| Batch channel sends | Same guarantees as current | `SmallVec` batching |
| Adaptive reserve sizing | Atomic load/store | `AtomicUsize` |

### Optimizations Requiring Care

#### Pre-serialize with mutable packet ID

FlashMQ modifies packet bytes in-place:
```cpp
p->setPacketId(packet_id);  // Writes 2 bytes at stored offset
```

**Safe Rust approaches:**

```rust
// Option A: Clone only mutable header (SAFE, ~95% of FlashMQ speed)
struct CachedPacket {
    header_template: [u8; 7],   // Fixed header + packet ID placeholder
    body: Bytes,                 // Immutable, shared
    packet_id_offset: usize,
}

impl CachedPacket {
    fn write_with_id(&self, buf: &mut BytesMut, packet_id: u16) {
        let mut header = self.header_template;
        header[self.packet_id_offset..][..2]
            .copy_from_slice(&packet_id.to_be_bytes());
        buf.extend_from_slice(&header);
        buf.extend_from_slice(&self.body);
    }
}

// Option B: Per-subscriber BytesMut from frozen Bytes (SAFE)
let frozen: Bytes = cached_packet.clone();  // Cheap Arc clone
let mut buf = BytesMut::from(frozen);        // Copy-on-write
buf[offset..offset+2].copy_from_slice(&packet_id.to_be_bytes());
```

**Unsafe approach (NOT recommended unless profiling proves necessary):**
```rust
// Option C: Direct mutation (UNSAFE - breaks Bytes invariants)
unsafe {
    let ptr = bytes.as_ptr().add(offset) as *mut u16;
    ptr.write(packet_id.to_be());
}
```

#### Direct write path

Bypassing channels requires careful synchronization:

```rust
impl Connection {
    fn try_direct_write(&self, packet: &[u8]) -> Result<(), QueuedPacket> {
        // try_lock avoids blocking but can fail
        match self.write_buf.try_lock() {
            Some(mut buf) => {
                buf.extend_from_slice(packet);
                Ok(())
            }
            None => Err(QueuedPacket(packet.to_vec())),  // Fall back to channel
        }
    }
}
```

**Risk:** Mixing direct writes with channel-based writes requires careful ordering to prevent out-of-order delivery. FlashMQ avoids this by being single-threaded per connection.

#### Per-node trie locks

More fine-grained locking increases deadlock risk:

```rust
// MUST maintain consistent lock ordering (parent → child)
fn traverse(&self, path: &[&str]) {
    let root = self.root.read();
    for segment in path {
        let child = root.children.get(segment)?;
        let child_guard = child.read();  // Must not hold root while acquiring child
        // ...
    }
}
```

### Safety vs Performance Trade-off

| Optimization | Safe Rust | Unsafe Rust | FlashMQ |
|--------------|-----------|-------------|---------|
| Arc-wrap Publish | 100% | — | 100% |
| Reuse HashMap | 100% | — | 100% |
| Pre-serialize (clone header) | 95% | — | 100% |
| Pre-serialize (in-place) | — | 100% | 100% |
| Direct writes | 80%* | 95% | 100% |
| Per-node locks | 90%** | — | 100% |

\* Requires fallback path for contention
\** More complex reasoning about correctness

### Recommended Approach

1. **Implement safe optimizations first** — Arc-wrap, thread-local HashMap, pre-serialize with header clone
2. **Measure** — Profile to see if remaining gap justifies complexity
3. **Consider targeted `unsafe`** — Only if profiling shows specific hot spots, with thorough documentation and testing

**Expected safe-only performance:** ~100-120ms latency (vs FlashMQ's 80ms)

This is a **2-2.5x improvement** over current VibeMQ while maintaining Rust's safety guarantees. The remaining 20-30% gap is the "cost" of memory safety — a worthwhile trade-off for most use cases.

### Licensing Note

FlashMQ uses the **Open Software License 3.0 (OSL-3.0)**, a copyleft license that requires:
- Source disclosure for network services (similar to AGPL)
- Patent grant provisions

VibeMQ's MIT/Apache-2.0 licensing is more permissive for commercial use. Any code directly derived from FlashMQ would need to comply with OSL-3.0 terms.

---

## Appendix: Key File Locations

### FlashMQ
| Component | File | Lines |
|-----------|------|-------|
| Packet caching | `publishcopyfactory.cpp` | 33-82 |
| Fan-out loop | `subscriptionstore.cpp` | 507-678 |
| In-place modification | `client.cpp` | 414-422 |
| Circular buffer | `cirbuf.cpp` | 72-90 |
| QoS queue | `qospacketqueue.cpp` | 152-165 |
| Event loop | `threadloop.cpp` | 50-139 |

### VibeMQ
| Component | File | Lines |
|-----------|------|-------|
| Fan-out (clone) | `broker/connection/publish.rs` | 259-353 |
| Dedup HashMap | `broker/connection/publish.rs` | 274 |
| Message clone | `broker/connection/publish.rs` | 311 |
| Trie matching | `topic/trie.rs` | 192-241 |
| QoS 2 state | `broker/connection/qos.rs` | 54-81 |
| Channel send | `broker/connection/mod.rs` | 235-238 |
