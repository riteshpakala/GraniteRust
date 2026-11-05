//! The installer, end to end: a package folder fetched, read with `cargo metadata`, built in release, described
//! and installed; called from the shelf; installed again; removed. Builds for a while, so it runs only with
//! `GRANITE_AGENT_BUILD=1`.

use std::sync::{Arc, Mutex};

use granite_agents::{AgentHost, AgentInstaller, AgentSource, RosterEntry};

fn building() -> bool {
    std::env::var("GRANITE_AGENT_BUILD").as_deref() == Ok("1")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_package_folder_is_installed_called_reinstalled_and_removed() {
    if !building() {
        eprintln!("skipped: set GRANITE_AGENT_BUILD=1 to build fixtures/hello-agent in release");
        return;
    }
    let shelf = tempfile::tempdir().unwrap();
    let hello = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hello-agent")
        .canonicalize()
        .unwrap();
    let stages = Arc::new(Mutex::new(Vec::new()));
    let kept = stages.clone();
    let installer = AgentInstaller::new(shelf.path());
    let installation = installer
        .install(
            AgentSource::Path {
                path: hello.to_string_lossy().into_owned(),
            },
            move |progress| {
                let mut stages = kept.lock().unwrap();
                if stages.last() != Some(&progress.stage) {
                    stages.push(progress.stage);
                }
            },
        )
        .await
        .unwrap();
    assert_eq!(installation.name, "hello-agent");
    assert_eq!(installation.words(), ["hello"]);
    assert!(
        installation
            .granite
            .as_deref()
            .is_some_and(|granite| granite.starts_with("0.1.0")),
        "{:?}",
        installation.granite
    );
    assert_eq!(
        *stages.lock().unwrap(),
        [
            "reading the package",
            "building hello-agent",
            "asking hello-agent what it carries",
            "installing hello-agent",
            "installed hello-agent: hello"
        ]
    );
    let left: Vec<String> = std::fs::read_dir(shelf.path().join(".builds"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        left,
        ["cargo-target"],
        "the work folder is gone; the shared target stays"
    );

    let notes = Arc::new(Mutex::new(Vec::new()));
    let heard = notes.clone();
    let host = AgentHost::none().with_note(move |line| heard.lock().unwrap().push(line.to_owned()));
    let Some(RosterEntry::Package(hello)) = installer
        .shelf
        .entries(std::env::vars().collect())
        .into_iter()
        .next()
    else {
        panic!("no package")
    };
    let answer: serde_json::Value =
        serde_json::from_slice(&hello.perform(br#"{"name":"Rao"}"#, host).await.unwrap()).unwrap();
    assert_eq!(answer["said"], "hello, Rao");
    let notes = notes.lock().unwrap().clone();
    assert_eq!(notes[0], "greeting Rao");
    assert!(
        notes[1].starts_with("released · peak"),
        "the last note is the cost: {notes:?}"
    );

    let again = installer
        .install(installation.source.clone(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        installer.shelf.installations().len(),
        1,
        "installing again replaces"
    );
    assert_eq!(again.name, "hello-agent");
    installer.remove("hello").unwrap();
    assert!(installer.shelf.installations().is_empty());
}
