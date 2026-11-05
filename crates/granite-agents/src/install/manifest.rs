//! What `cargo metadata` says of a package, as an install needs it: its name, the bin that is its runner, and the
//! granite-agents it is built against.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::AgentInstallError;

pub(crate) struct CargoManifest {
    /// The installation's name: `[package.metadata.granite] name`, else the package folder's name.
    pub name: String,
    /// The Cargo package that holds the runner.
    pub package: String,
    pub bin: String,
    pub manifest_path: PathBuf,
    pub granite: Option<String>,
}

impl CargoManifest {
    pub fn read(
        metadata: &str,
        root: &Path,
        source: &str,
    ) -> Result<CargoManifest, AgentInstallError> {
        let not_a_package = |reason: &str| AgentInstallError::NotAPackage {
            source: source.into(),
            reason: reason.into(),
        };
        let metadata: Value = serde_json::from_str(metadata)
            .map_err(|_| not_a_package("`cargo metadata` said nothing that reads"))?;
        let members: Vec<&str> = metadata["workspace_members"]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
        let ours: Vec<&Value> = packages
            .iter()
            .filter(|package| members.contains(&package["id"].as_str().unwrap_or("")))
            .collect();
        let named = |package: &&&Value| package["metadata"]["granite"]["agent"].is_string();
        let root_manifest = root.join("Cargo.toml");
        let package = ours
            .iter()
            .find(named)
            .or_else(|| ours.iter().find(|package| package["manifest_path"].as_str().map(Path::new) == Some(root_manifest.as_path())))
            .or_else(|| if ours.len() == 1 { ours.first() } else { None })
            .ok_or_else(|| not_a_package("no package in it names its runner (`[package.metadata.granite] agent = \"<bin>\"`)"))?;
        let package_name = package["name"].as_str().unwrap_or_default().to_owned();
        let bins: Vec<&str> = package["targets"]
            .as_array()
            .map(|targets| {
                targets
                    .iter()
                    .filter(|target| {
                        target["kind"]
                            .as_array()
                            .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
                    })
                    .filter_map(|target| target["name"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        let folder = root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| package_name.clone());
        let name = package["metadata"]["granite"]["name"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or(folder);
        let bin = match package["metadata"]["granite"]["agent"].as_str() {
            Some(bin) if bins.contains(&bin) => bin.to_owned(),
            Some(bin) => {
                return Err(not_a_package(&format!(
                    "its runner `{bin}` is not one of its bins"
                )));
            }
            None if bins.contains(&"agent") => "agent".to_owned(),
            None if bins.len() == 1 => bins[0].to_owned(),
            None => return Err(AgentInstallError::NoAgent { name }),
        };
        let id = package["id"].as_str().unwrap_or_default();
        let node = metadata["resolve"]["nodes"]
            .as_array()
            .and_then(|nodes| nodes.iter().find(|node| node["id"] == id));
        let dependency_ids: Vec<&str> = node
            .and_then(|node| node["deps"].as_array())
            .map(|deps| deps.iter().filter_map(|dep| dep["pkg"].as_str()).collect())
            .unwrap_or_default();
        let granite = packages
            .iter()
            .find(|candidate| {
                candidate["name"] == "granite-agents"
                    && dependency_ids.contains(&candidate["id"].as_str().unwrap_or(""))
            })
            .ok_or_else(|| AgentInstallError::NotGranite { name: name.clone() })?;
        let version = granite["version"].as_str().unwrap_or("?");
        let revision = granite["source"]
            .as_str()
            .and_then(|source| source.split('#').nth(1));
        let granite = Some(match revision {
            Some(revision) => format!("{version}@{revision}"),
            None => version.to_owned(),
        });
        let manifest_path = PathBuf::from(package["manifest_path"].as_str().unwrap_or_default());
        Ok(CargoManifest {
            name,
            package: package_name,
            bin,
            manifest_path,
            granite,
        })
    }
}
