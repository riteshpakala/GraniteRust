//! A shelf of runners, installed by hand: read from its records, called through them, and refused once changed.

use std::io::Write;
use std::path::Path;

use granite_agents::{
    AgentHost, AgentInstallation, AgentProcess, AgentRemoteError, AgentShelf, AgentSource,
    RosterEntry, install,
};

const PARROT: &str = env!("CARGO_BIN_EXE_parrot-runner");

async fn install_by_hand(shelf: &Path, name: &str) -> AgentInstallation {
    let folder = shelf.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let executable = folder.join("agent");
    std::fs::copy(PARROT, &executable).unwrap();
    let metadata = std::fs::metadata(&executable).unwrap();
    let modified = metadata
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let installation = AgentInstallation {
        format: AgentInstallation::FORMAT.into(),
        name: name.into(),
        source: AgentSource::Path {
            path: "/fixtures/parrot".into(),
        },
        revision: None,
        granite: Some("0.1.0".into()),
        executable: "agent".into(),
        sha256: AgentShelf::sha256(&executable).unwrap(),
        size: metadata.len(),
        modified: Some(modified),
        built_at: install::now_iso8601(),
        entries: AgentProcess::new(&executable).describe().await.unwrap(),
    };
    std::fs::write(
        folder.join(AgentInstallation::FILE),
        granite_agents::json::pretty(&installation).unwrap(),
    )
    .unwrap();
    installation
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shelf_offers_what_its_records_say_and_calls_through_them() {
    let shelf = tempfile::tempdir().unwrap();
    let stamp = AgentShelf::new(shelf.path()).stamp();
    install_by_hand(shelf.path(), "ParrotAgent").await;
    let read = AgentShelf::new(shelf.path());
    assert_ne!(read.stamp(), stamp, "an installation changes the stamp");
    assert_eq!(read.installations().len(), 1);
    assert_eq!(
        read.installation("PARROT").map(|found| found.name),
        Some("ParrotAgent".to_owned()),
        "found by its word, in any case"
    );
    let entries = read.entries(std::env::vars().collect());
    let Some(RosterEntry::Package(parrot)) = entries.first() else {
        panic!("no package")
    };
    let bytes = parrot
        .perform(br#"{"text":"from the shelf"}"#, AgentHost::none())
        .await
        .unwrap();
    assert!(String::from_utf8(bytes).unwrap().contains("from the shelf"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_executable_is_refused() {
    let shelf = tempfile::tempdir().unwrap();
    let installation = install_by_hand(shelf.path(), "ParrotAgent").await;
    let read = AgentShelf::new(shelf.path());
    read.verify(&installation).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(read.executable_of(&installation))
        .unwrap()
        .write_all(&[0])
        .unwrap();
    let Err(AgentRemoteError::Changed { word, .. }) = read.verify(&installation) else {
        panic!("an appended byte passed")
    };
    assert_eq!(word, "parrot");
    let Some(RosterEntry::Package(parrot)) = read.entries(Default::default()).into_iter().next()
    else {
        panic!("no package")
    };
    let error = parrot
        .perform(br#"{"text":"x"}"#, AgentHost::none())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("changed since it was installed"),
        "{error}"
    );
}

#[test]
fn a_record_that_does_not_read_is_said_and_left_out() {
    let shelf = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(shelf.path().join("Broken")).unwrap();
    std::fs::write(
        shelf.path().join("Broken").join(AgentInstallation::FILE),
        "{",
    )
    .unwrap();
    std::fs::create_dir_all(shelf.path().join(".builds")).unwrap();
    let read = AgentShelf::new(shelf.path());
    assert!(read.installations().is_empty());
    let problems = read.problems();
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].starts_with("Broken: its installation.json does not read"),
        "{problems:?}"
    );
}

#[test]
fn a_source_is_a_repository_at_a_revision_or_a_folder() {
    let base = Path::new("/work");
    let rows = [
        (
            "https://github.com/rao/ComicAgent.git",
            AgentSource::Git {
                url: "https://github.com/rao/ComicAgent.git".into(),
                revision: None,
            },
        ),
        (
            "https://github.com/rao/ComicAgent.git@f9bdda7",
            AgentSource::Git {
                url: "https://github.com/rao/ComicAgent.git".into(),
                revision: Some("f9bdda7".into()),
            },
        ),
        (
            "git@github.com:rao/ComicAgent.git",
            AgentSource::Git {
                url: "git@github.com:rao/ComicAgent.git".into(),
                revision: None,
            },
        ),
        (
            "/abs/ComicAgent",
            AgentSource::Path {
                path: "/abs/ComicAgent".into(),
            },
        ),
        (
            "./agents/../ComicAgent",
            AgentSource::Path {
                path: "/work/ComicAgent".into(),
            },
        ),
    ];
    for (said, source) in rows {
        assert_eq!(AgentSource::parse(said, base), source, "{said}");
    }
    assert_eq!(
        AgentSource::parse("https://x/y.git@abc", base)
            .newest()
            .to_string(),
        "https://x/y.git"
    );
}
