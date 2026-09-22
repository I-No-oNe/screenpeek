use std::process::Command;

#[test]
fn invalid_input_fails_before_touching_the_desktop() {
    for args in [
        vec!["scan", "--region", "0,0,0,10"],
        vec!["read", "does-not-exist.png"],
        vec!["read", "does-not-exist.png", "--scale", "0"],
        vec!["read", "does-not-exist.png", "--scale", "4294967295"],
        vec!["click", "Save", "--button", "invalid"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_screenpeek"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn no_daemon_does_not_connect_to_an_existing_endpoint() {
    use std::io::ErrorKind;
    use std::net::TcpListener;
    let cache = tempfile::tempdir().unwrap();
    let directory = cache.path().join("screenpeek");
    std::fs::create_dir(&directory).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::write(
        directory.join("daemon-v2"),
        format!("{} token", listener.local_addr().unwrap().port()),
    )
    .unwrap();
    let endpoint = directory.join("daemon-v2");
    let mut child = Command::new(env!("CARGO_BIN_EXE_screenpeek"))
        .arg("scan")
        .env("SCREENPEEK_NO_DAEMON", "1")
        .env("XDG_CACHE_HOME", cache.path())
        .env("DISPLAY", "invalid")
        .env("WAYLAND_DISPLAY", "invalid")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        if std::time::Instant::now() >= deadline {
            std::fs::remove_file(endpoint).unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("daemon bypass hung");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
}
