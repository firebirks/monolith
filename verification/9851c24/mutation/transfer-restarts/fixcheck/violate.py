"""Applies a deliberate violation of the priority of a durable chat to
crates/monolith-core/src/runtime.rs of a scratch copy (never to the
repository), for the check of the corrected test.

v1: a session sends no chat while a file comes in: the chat waits behind
    the rest of the transfer.
v2: a session that has a durable chat ready while a file comes in sends
    it only at the third top of its loop: two frames later, one more
    acknowledgement.

Usage: violate.py <copy> v1|v2"""
import sys

copy, kind = sys.argv[1], sys.argv[2]
path = f"{copy}/crates/monolith-core/src/runtime.rs"
text = open(path, encoding="utf-8", newline="").read()
if kind == "v1":
    old = """                if running.in_flight.is_none()
                    && runtime.manager.is_preferred(&contact, running.serial)
                {"""
    new = """                if running.in_flight.is_none()
                    && runtime.manager.is_preferred(&contact, running.serial)
                    && running.files.exchange.incoming().is_empty()
                {"""
elif kind == "v2":
    old = """                    if let Some((_, id, text)) = next.ok().flatten() {"""
    new = """                    static SKIPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let next = next.ok().flatten().filter(|_| {
                        running.files.exchange.incoming().is_empty()
                            || SKIPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 2
                    });
                    if next.is_some() {
                        SKIPS.store(0, std::sync::atomic::Ordering::Relaxed);
                    }
                    if let Some((_, id, text)) = next {"""
elif kind == "v3":
    # A durable chat ready while a file comes in waits until the session
    # has read two more frames: it leaves behind the next acknowledgement.
    old = """                    if let Some((_, id, text)) = next.ok().flatten() {"""
    new = """                    let next = next.ok().flatten().filter(|_| {
                        running.files.exchange.incoming().is_empty() || {
                            V3_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
                            V3_FRAMES.load(std::sync::atomic::Ordering::Relaxed) >= 2
                        }
                    });
                    if next.is_some() {
                        V3_PENDING.store(false, std::sync::atomic::Ordering::Relaxed);
                        V3_FRAMES.store(0, std::sync::atomic::Ordering::Relaxed);
                    }
                    if let Some((_, id, text)) = next {"""
    old2 = """                Next::Received(Ok(arrived)) => {
                    if self.pace(runtime, running, &arrived).await == Waited::Stop {"""
    new2 = """                Next::Received(Ok(arrived)) => {
                    if V3_PENDING.load(std::sync::atomic::Ordering::Relaxed) {
                        V3_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    if self.pace(runtime, running, &arrived).await == Waited::Stop {"""
    assert text.count(old2) == 1, f"{kind}: second pattern found {text.count(old2)} times"
    text = text.replace(old2, new2, 1)
    text += """
static V3_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static V3_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
"""
else:
    sys.exit("v1, v2 or v3")
assert text.count(old) == 1, f"{kind}: pattern found {text.count(old)} times"
open(path, "w", encoding="utf-8", newline="").write(text.replace(old, new, 1))
print(f"{kind} applied to {path}")
