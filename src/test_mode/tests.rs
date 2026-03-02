use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use camino::Utf8PathBuf;

use super::*;
use crate::clock::Clock;
use crate::narrate::audio::AudioSource;
use crate::narrate::clipboard_capture::ClipboardSource;
use crate::narrate::editor_capture::EditorStateSource;
use crate::narrate::ext_capture::{ExternalSnapshot, ExternalSource};
use crate::narrate::transcribe::stub::Injection;
use crate::state;

// -- Serde round-trip -----------------------------------------------------

/// Inject variants survive JSON serialization round-trip.
#[test]
fn inject_serde_round_trip() {
    let cases = vec![
        Inject::AdvanceTime { duration_ms: 100 },
        Inject::Speech {
            text: "hello world".into(),
            duration_ms: 2000,
        },
        Inject::Silence { duration_ms: 500 },
        Inject::EditorState {
            files: vec![state::FileEntry {
                path: Utf8PathBuf::from("/tmp/foo.rs"),
                selections: vec![],
            }],
        },
        Inject::ExternalSelection {
            app: "Safari".into(),
            text: "selected text".into(),
        },
        Inject::Clipboard {
            text: "clipboard content".into(),
        },
    ];
    for msg in &cases {
        let json = serde_json::to_string(msg).unwrap();
        let _round_tripped: Inject = serde_json::from_str(&json).unwrap();
    }
}

/// Handshake survives JSON serialization round-trip.
#[test]
fn handshake_serde_round_trip() {
    let hs = Handshake {
        pid: 12345,
        argv: vec!["attend".into(), "narrate".into(), "_daemon".into()],
    };
    let json = serde_json::to_string(&hs).unwrap();
    let rt: Handshake = serde_json::from_str(&json).unwrap();
    assert_eq!(rt.pid, 12345);
    assert_eq!(rt.argv, vec!["attend", "narrate", "_daemon"]);
}

// -- Stub capture sources -------------------------------------------------

/// StubEditorSource returns the most recently injected state.
#[test]
fn stub_editor_source_returns_injected_state() {
    let shared: Arc<Mutex<Option<EditorState>>> = Arc::default();
    let stub = stubs::StubEditorSource::new(Arc::clone(&shared));

    // Initially empty.
    assert!(stub.current(None, &[]).unwrap().is_none());

    // Inject a state.
    *shared.lock().unwrap() = Some(EditorState {
        files: vec![state::FileEntry {
            path: Utf8PathBuf::from("/tmp/test.rs"),
            selections: vec![],
        }],
        cwd: None,
    });

    let state = stub.current(None, &[]).unwrap().unwrap();
    assert_eq!(state.files.len(), 1);
    assert_eq!(state.files[0].path, "/tmp/test.rs");

    // Returns the same state on repeated polls (latest-wins).
    let state2 = stub.current(None, &[]).unwrap().unwrap();
    assert_eq!(state2.files[0].path, "/tmp/test.rs");
}

/// StubClipboardSource returns injected text, never images.
#[test]
fn stub_clipboard_source_returns_injected_text() {
    let shared: Arc<Mutex<Option<String>>> = Arc::default();
    let mut stub = stubs::StubClipboardSource::new(Arc::clone(&shared));

    assert!(stub.get_text().is_none());
    assert!(stub.get_image().is_none());

    *shared.lock().unwrap() = Some("pasted".into());

    assert_eq!(stub.get_text().unwrap(), "pasted");
    assert!(stub.get_image().is_none()); // Never returns images.
}

/// StubExternalSource returns injected snapshot and reports available.
#[test]
fn stub_external_source_returns_injected_snapshot() {
    let shared: Arc<Mutex<Option<ExternalSnapshot>>> = Arc::default();
    let stub = stubs::StubExternalSource::new(Arc::clone(&shared));

    assert!(stub.is_available());
    assert!(stub.query().is_none());

    *shared.lock().unwrap() = Some(ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "shell".into(),
        selected_text: Some("hello".into()),
    });

    let snap = stub.query().unwrap();
    assert_eq!(snap.app, "iTerm2");
    assert_eq!(snap.selected_text.unwrap(), "hello");
}

/// StubAudioSource produces a tiny chunk when not paused, nothing when paused.
#[test]
fn stub_audio_source_produces_chunks_when_active() {
    let clock = Arc::new(crate::clock::MockClock::new(chrono::DateTime::UNIX_EPOCH));
    let mut stub = stubs::StubAudioSource::new(16000, clock);

    assert_eq!(stub.sample_rate(), 16000);

    // Active: produces a chunk.
    let chunks = stub.take_chunks();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].samples, vec![0.0]);

    // Drain also produces a chunk.
    assert_eq!(stub.drain().chunks.len(), 1);

    // Paused: produces nothing.
    assert!(stub.pause().is_ok());
    assert!(stub.take_chunks().is_empty());
    assert!(stub.drain().chunks.is_empty());

    // Resumed: produces again.
    assert!(stub.resume().is_ok());
    assert_eq!(stub.take_chunks().len(), 1);

    // Stop always returns empty.
    assert!(stub.stop().chunks.is_empty());
}

// -- InjectRouter dispatch ------------------------------------------------

/// InjectRouter dispatches Speech to the transcriber channel.
#[test]
fn router_dispatches_speech() {
    let (tx, rx) = std::sync::mpsc::channel();
    let router = InjectRouter {
        transcriber_tx: tx,
        editor_state: Arc::default(),
        ext_snapshot: Arc::default(),
        clipboard_text: Arc::default(),
    };

    router.dispatch(Inject::Speech {
        text: "hello".into(),
        duration_ms: 1000,
    });

    let inj = rx.try_recv().unwrap();
    assert_eq!(inj.text, "hello");
    assert_eq!(inj.duration_ms, 1000);
}

/// InjectRouter dispatches Silence as empty-text Injection.
#[test]
fn router_dispatches_silence() {
    let (tx, rx) = std::sync::mpsc::channel();
    let router = InjectRouter {
        transcriber_tx: tx,
        editor_state: Arc::default(),
        ext_snapshot: Arc::default(),
        clipboard_text: Arc::default(),
    };

    router.dispatch(Inject::Silence { duration_ms: 500 });

    let inj = rx.try_recv().unwrap();
    assert_eq!(inj.text, "");
    assert_eq!(inj.duration_ms, 500);
}

/// InjectRouter dispatches EditorState to the shared mutex.
#[test]
fn router_dispatches_editor_state() {
    let editor_state: Arc<Mutex<Option<EditorState>>> = Arc::default();
    let router = InjectRouter {
        transcriber_tx: std::sync::mpsc::channel::<Injection>().0,
        editor_state: Arc::clone(&editor_state),
        ext_snapshot: Arc::default(),
        clipboard_text: Arc::default(),
    };

    router.dispatch(Inject::EditorState {
        files: vec![state::FileEntry {
            path: Utf8PathBuf::from("/src/main.rs"),
            selections: vec![],
        }],
    });

    let state = editor_state.lock().unwrap();
    let files = &state.as_ref().unwrap().files;
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/src/main.rs");
}

/// InjectRouter dispatches ExternalSelection to the shared mutex.
#[test]
fn router_dispatches_external_selection() {
    let ext: Arc<Mutex<Option<ExternalSnapshot>>> = Arc::default();
    let router = InjectRouter {
        transcriber_tx: std::sync::mpsc::channel::<Injection>().0,
        editor_state: Arc::default(),
        ext_snapshot: Arc::clone(&ext),
        clipboard_text: Arc::default(),
    };

    router.dispatch(Inject::ExternalSelection {
        app: "Safari".into(),
        text: "selected".into(),
    });

    let snap = ext.lock().unwrap().clone().unwrap();
    assert_eq!(snap.app, "Safari");
    assert_eq!(snap.selected_text.unwrap(), "selected");
}

/// InjectRouter dispatches Clipboard text to the shared mutex.
#[test]
fn router_dispatches_clipboard() {
    let clip: Arc<Mutex<Option<String>>> = Arc::default();
    let router = InjectRouter {
        transcriber_tx: std::sync::mpsc::channel::<Injection>().0,
        editor_state: Arc::default(),
        ext_snapshot: Arc::default(),
        clipboard_text: Arc::clone(&clip),
    };

    router.dispatch(Inject::Clipboard {
        text: "copied".into(),
    });

    assert_eq!(clip.lock().unwrap().as_deref(), Some("copied"));
}

// -- init() + inject socket integration -----------------------------------

/// Full round-trip: init() connects to inject socket, sends handshake,
/// background reader processes AdvanceTime and advances the MockClock.
#[test]
fn init_connects_and_processes_inject_messages() {
    let guard = state::CacheDirGuard::new();
    let sock_path = guard.cache.join("test-inject.sock");

    // Bind the inject socket (harness role).
    let listener = UnixListener::bind(sock_path.as_std_path()).unwrap();

    // init() creates clock + router; connect() sends handshake + spawns reader.
    init();
    connect();

    // Accept the connection (test thread acting as harness, not
    // clock-managed).
    #[allow(clippy::disallowed_methods)]
    let (stream, _) = listener.accept().unwrap();
    let mut reader = BufReader::new(&stream);

    // Read the handshake.
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let hs: Handshake = serde_json::from_str(&line).unwrap();
    assert_eq!(hs.pid, std::process::id());
    assert!(!hs.argv.is_empty());

    // Verify the MockClock was created (starts at epoch).
    let clock = super::clock().expect("MockClock not set");
    assert_eq!(clock.now(), chrono::DateTime::UNIX_EPOCH);

    // Get a SyncClock for sleeping. Using for_thread() gives us a
    // ParticipantMockClock with proper departure tracking.
    let sleeper = clock.for_thread();

    // Helper: send an inject message and wait for the clock to advance.
    //
    // The sleeper blocks on the condvar until advance() meets the
    // deadline. This isn't circular: the *background reader thread*
    // calls advance(), not this thread. The deadlock invariant
    // ("the inject reader must never call clock.sleep()") is respected.
    let send_and_wait = |msg: &Inject, wait: Duration| {
        let mut json = serde_json::to_vec(msg).unwrap();
        json.push(b'\n');
        (&stream).write_all(&json).unwrap();
        sleeper.sleep(wait);
    };

    let epoch = chrono::DateTime::UNIX_EPOCH;
    send_and_wait(
        &Inject::AdvanceTime { duration_ms: 5000 },
        Duration::from_secs(5),
    );
    assert_eq!(clock.now(), epoch + Duration::from_secs(5));

    send_and_wait(
        &Inject::AdvanceTime { duration_ms: 3000 },
        Duration::from_secs(3),
    );
    assert_eq!(clock.now(), epoch + Duration::from_secs(8));
}
