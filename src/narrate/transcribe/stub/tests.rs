use super::*;

/// Injected words are returned with evenly-spaced timestamps.
#[test]
fn synthetic_timestamps_are_evenly_spaced() {
    let (mut stub, tx) = StubTranscriber::new();

    tx.send(Injection {
        text: "hello world".into(),
        duration_ms: 2000,
    })
    .unwrap();

    let words = stub.transcribe(&[]).unwrap();
    assert_eq!(words.len(), 2);
    assert_eq!(words[0].text, "hello");
    assert_eq!(words[1].text, "world");

    // 2s / 2 words = 1s per word
    assert!((words[0].start_secs - 0.0).abs() < 1e-9);
    assert!((words[0].end_secs - 1.0).abs() < 1e-9);
    assert!((words[1].start_secs - 1.0).abs() < 1e-9);
    assert!((words[1].end_secs - 2.0).abs() < 1e-9);
}

/// Multiple injections are drained in a single transcribe() call
/// with cumulative time offsets.
#[test]
fn multiple_injections_accumulate_time() {
    let (mut stub, tx) = StubTranscriber::new();

    tx.send(Injection {
        text: "first".into(),
        duration_ms: 1000,
    })
    .unwrap();
    tx.send(Injection {
        text: "second".into(),
        duration_ms: 1000,
    })
    .unwrap();

    let words = stub.transcribe(&[]).unwrap();
    assert_eq!(words.len(), 2);
    assert_eq!(words[0].text, "first");
    assert_eq!(words[1].text, "second");

    // first: 0-1s, second: 1-2s
    assert!((words[0].start_secs - 0.0).abs() < 1e-9);
    assert!((words[1].start_secs - 1.0).abs() < 1e-9);
}

/// No pending injections returns empty (silence).
#[test]
fn no_injections_returns_empty() {
    let (mut stub, _tx) = StubTranscriber::new();
    let words = stub.transcribe(&[]).unwrap();
    assert!(words.is_empty());
}

/// Whitespace-only injection text produces no words but advances time.
#[test]
fn whitespace_only_injection_advances_time() {
    let (mut stub, tx) = StubTranscriber::new();

    tx.send(Injection {
        text: "   ".into(),
        duration_ms: 1000,
    })
    .unwrap();
    tx.send(Injection {
        text: "after".into(),
        duration_ms: 1000,
    })
    .unwrap();

    let words = stub.transcribe(&[]).unwrap();
    assert_eq!(words.len(), 1);
    assert_eq!(words[0].text, "after");
    // Whitespace injection consumed 1s, so "after" starts at 1s.
    assert!((words[0].start_secs - 1.0).abs() < 1e-9);
}

/// Dropped sender doesn't cause errors — just returns whatever
/// was buffered.
#[test]
fn dropped_sender_is_graceful() {
    let (mut stub, tx) = StubTranscriber::new();

    tx.send(Injection {
        text: "buffered".into(),
        duration_ms: 500,
    })
    .unwrap();
    drop(tx);

    let words = stub.transcribe(&[]).unwrap();
    assert_eq!(words.len(), 1);
    assert_eq!(words[0].text, "buffered");

    // Second call after sender dropped: empty, no error.
    let words = stub.transcribe(&[]).unwrap();
    assert!(words.is_empty());
}
