// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::os::unix::net::UnixListener as StdUnixListener;
use std::process;

use crate::identity::daemon_own_uid;
use crate::test_support::TestDir;

use super::*;

/// A path one byte past what this platform's `sockaddr_un` can hold.
fn oversized_socket_path() -> PathBuf {
    PathBuf::from(format!("/tmp/{}", "a".repeat(MAX_ADDRESS_LEN)))
}

#[test]
fn max_address_len_leaves_exactly_one_byte_for_the_nul_terminator() {
    let sun_path_capacity =
        size_of::<libc::sockaddr_un>() - offset_of!(libc::sockaddr_un, sun_path);
    assert_eq!(MAX_ADDRESS_LEN, sun_path_capacity - 1);

    // Guards the derivation itself: every supported Unix sits in this
    // range (108 on Linux, 104 on macOS and the BSDs), so a wildly
    // different answer means `sun_path` stopped being the final field
    // rather than that a platform genuinely disagrees.
    assert!(
        (100..=110).contains(&sun_path_capacity),
        "implausible sun_path capacity {sun_path_capacity}"
    );
}

#[test]
fn bind_rejects_a_socket_path_longer_than_the_platform_allows() {
    let path = oversized_socket_path();
    let Err(error) = Listener::bind(&Address::Unix(path.clone())) else {
        panic!("an oversized socket path must not bind");
    };

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    let message = error.to_string();
    assert!(
        message.contains(&MAX_ADDRESS_LEN.to_string()),
        "error should name the limit it broke: {message}"
    );

    // Rejected before anything touched the filesystem: the parent
    // directory is never created, nor a socket left behind.
    assert!(!path.exists());
}

#[tokio::test]
async fn connect_rejects_a_socket_path_longer_than_the_platform_allows() {
    let Err(error) = connect(&Address::Unix(oversized_socket_path())).await else {
        panic!("an oversized socket path must not connect");
    };

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_socket_path_at_exactly_the_limit_is_accepted() {
    // The boundary the NUL byte creates: one byte shorter must pass
    // validation, so the check is not off by one in the strict
    // direction either.
    let path = PathBuf::from(format!("/tmp/{}", "a".repeat(MAX_ADDRESS_LEN - 5)));
    assert_eq!(path.as_os_str().as_bytes().len(), MAX_ADDRESS_LEN);
    validate_address_len(&path).expect("a path at exactly the limit should be accepted");
}

#[test]
fn prepare_socket_path_creates_private_runtime_directory() {
    let runtime_dir = TestDir::uncreated("create-private");
    let path = runtime_dir.join("luminated.sock");

    prepare_socket_path(&path).expect("prepare socket path");

    let mode = fs::metadata(&runtime_dir)
        .expect("read runtime directory metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, PRIVATE_RUNTIME_DIR_MODE);
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn prepare_socket_path_rejects_group_writable_parent() {
    let runtime_dir = TestDir::new("unsafe-parent");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o770))
        .expect("set unsafe permissions");

    let error = prepare_socket_path(&runtime_dir.join("luminated.sock"))
        .expect_err("group-writable parent must be rejected");
    assert!(error.to_string().contains("must not be writable"));
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn prepare_socket_path_removes_a_stale_socket_left_by_a_previous_run() {
    let runtime_dir = TestDir::new("stale-socket");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    let path = runtime_dir.join("luminated.sock");
    // A socket left behind by a daemon that didn't shut down cleanly
    // (e.g. `kill -9`) must be cleared out of the way rather than
    // treated as an already-listening daemon.
    let stale = StdUnixListener::bind(&path).expect("bind stale socket");
    drop(stale);
    assert!(path.exists(), "the stale socket file should remain on disk");

    prepare_socket_path(&path).expect("stale socket should be cleared");
    assert!(!path.exists());

    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn prepare_socket_path_refuses_non_socket_node() {
    let runtime_dir = TestDir::new("non-socket");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    let path = runtime_dir.join("luminated.sock");
    fs::File::create(&path).expect("create regular file");

    let error = prepare_socket_path(&path).expect_err("regular file must be preserved");
    assert!(error.to_string().contains("refusing to replace non-socket"));
    assert!(path.is_file());

    fs::remove_file(path).expect("remove regular file");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn socket_path_helpers_reject_paths_without_safe_directory_parents() {
    let no_parent = Path::new("luminated.sock");
    assert!(prepare_socket_path(no_parent).is_err());
    assert!(validate_socket_parent_owner(no_parent).is_err());

    let runtime_dir = TestDir::new("parent-is-file");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    let parent = runtime_dir.join("not-a-directory");
    fs::File::create(&parent).expect("create parent file");
    let error = prepare_socket_path(&parent.join("luminated.sock"))
        .expect_err("socket parent must be a directory");
    assert!(error.to_string().contains("not a directory"));
    fs::remove_file(parent).expect("remove parent file");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn socket_permission_helper_sets_explicit_mode() {
    let runtime_dir = TestDir::new("permissions");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    let path = runtime_dir.join("luminated.sock");
    fs::File::create(&path).expect("create test filesystem node");

    set_socket_permissions(&path).expect("set socket permissions");
    let mode = fs::metadata(&path)
        .expect("read socket metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, SOCKET_MODE);

    fs::remove_file(path).expect("remove test node");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn real_socket_identity_owner_validation_and_cleanup_round_trip() {
    let runtime_dir = TestDir::new("socket-lifecycle");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
        .expect("secure runtime directory");
    let path = runtime_dir.join("luminated.sock");

    let listener = StdUnixListener::bind(&path).expect("bind unix socket");
    validate_socket_parent_owner(&path).expect("socket and parent should share an owner");
    let identity = SocketIdentity::from_path(&path).expect("capture socket identity");
    remove_owned_socket(&path, identity).expect("remove owned socket");
    assert!(!path.exists());
    remove_owned_socket(&path, identity).expect("missing owned socket is already clean");

    // Keep the original socket open, as Listener::drop does while checking
    // the path. Otherwise its freed inode may be reused for the replacement.
    let replacement = StdUnixListener::bind(&path).expect("bind replacement unix socket");
    let replacement_identity =
        SocketIdentity::from_path(&path).expect("capture replacement identity");
    assert_ne!(identity, replacement_identity);
    let error = remove_owned_socket(&path, identity)
        .expect_err("identity mismatch must preserve replacement socket");
    assert!(error.to_string().contains("replaced socket path"));
    drop(replacement);
    remove_owned_socket(&path, replacement_identity).expect("remove replacement socket");
    drop(listener);
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn shutdown_cleanup_preserves_replacement_node() {
    let runtime_dir = TestDir::new("replacement");
    fs::create_dir_all(&runtime_dir).expect("create runtime directory");
    let path = runtime_dir.join("luminated.sock");
    let original_path = runtime_dir.join("original");
    fs::File::create(&original_path).expect("create original node");
    let metadata = fs::metadata(&original_path).expect("read original identity");
    let identity = SocketIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    fs::File::create(&path).expect("create replacement node");

    let error = remove_owned_socket(&path, identity)
        .expect_err("cleanup must not remove a replacement filesystem node");
    assert!(error.to_string().contains("is not a socket"));
    assert!(path.exists());

    fs::remove_file(path).expect("remove replacement node");
    fs::remove_file(original_path).expect("remove original node");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[tokio::test]
async fn bind_creates_a_group_writable_socket_owned_by_the_current_process() {
    let runtime_dir = TestDir::uncreated("bind-owns-and-modes");
    let path = runtime_dir.join("luminated.sock");
    let address = Address::Unix(path.clone());

    let listener = Listener::bind(&address).expect("bind listener");

    let mode = fs::metadata(&path).expect("stat socket").mode() & 0o7777;
    assert_eq!(mode, SOCKET_MODE);
    validate_socket_parent_owner(&path).expect("socket and parent should share an owner");

    drop(listener);
    assert!(!path.exists(), "listener drop should remove its own socket");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[tokio::test]
async fn bind_refuses_to_remove_a_socket_replaced_by_another_process() {
    let runtime_dir = TestDir::uncreated("bind-replaced");
    let path = runtime_dir.join("luminated.sock");
    let address = Address::Unix(path.clone());

    let owned = Listener::bind(&address).expect("bind owned listener");
    fs::remove_file(&path).expect("simulate the socket being removed out from under us");
    let _replacement = StdUnixListener::bind(&path).expect("bind replacement socket");

    drop(owned);
    assert!(
        path.exists(),
        "drop must not remove a socket it didn't create"
    );
    fs::remove_file(&path).expect("remove replacement socket");
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[tokio::test]
async fn accept_captures_peer_credentials_and_yields_a_connection() {
    let runtime_dir = TestDir::uncreated("accept-credentials");
    let path = runtime_dir.join("luminated.sock");
    let address = Address::Unix(path.clone());
    let mut listener = Listener::bind(&address).expect("bind listener");

    let connect_path = path.clone();
    let client_task =
        tokio::spawn(async move { UnixStream::connect(&connect_path).await.expect("connect") });

    let (_connection, credential) = listener.accept().await.expect("accept connection");
    let _client = client_task.await.expect("client task");

    match credential {
        PeerCredential::Unix { uid, pid, .. } => {
            assert_eq!(uid, daemon_own_uid());
            assert_eq!(pid, Some(process::id()));
        }
        PeerCredential::Windows { .. } => {
            unreachable!("Unix accept must yield PeerCredential::Unix")
        }
    }
    drop(listener);
    fs::remove_dir(runtime_dir).expect("remove runtime directory");
}

#[test]
fn transient_accept_errors_are_recognized_and_others_are_not() {
    for errno in [libc::EMFILE, libc::ENFILE, libc::ENOBUFS, libc::ENOMEM] {
        assert!(is_transient_accept_error(&io::Error::from_raw_os_error(
            errno
        )));
    }
    assert!(!is_transient_accept_error(&io::Error::from_raw_os_error(
        libc::EINVAL
    )));
    assert!(!is_transient_accept_error(&io::Error::other("no errno")));
}
