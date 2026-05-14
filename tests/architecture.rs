#[test]
fn no_thread_local_state() {
    let output = std::process::Command::new("grep")
        .args(["-r", "thread_local!", "src/"])
        .output()
        .expect("grep failed");
    let matches = String::from_utf8_lossy(&output.stdout);
    assert!(
        matches.is_empty(),
        "Found thread_local! usage in src/. All state must be in the ownership hierarchy:\n{matches}"
    );
}
