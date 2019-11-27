use crate::{errors::*, find, LocalManifest};
use std::path::{Path, PathBuf};

/// A collection of manifests.
#[derive(Debug)]
pub struct Manifests(pub Vec<(LocalManifest, cargo_metadata::Package)>);

impl Manifests {
    /// Get all manifests in the workspace.
    pub fn get_all(manifest_path: &Option<PathBuf>) -> Result<Self> {
        let mut cmd = cargo_metadata::MetadataCommand::new();
        cmd.no_deps();
        if let Some(path) = manifest_path {
            cmd.manifest_path(path);
        }
        let result = cmd
            .exec()
            .chain_err(|| "Failed to get workspace metadata")?;
        result
            .packages
            .into_iter()
            .map(|package| {
                Ok((
                    LocalManifest::try_new(Path::new(&package.manifest_path))?,
                    package,
                ))
            })
            .collect::<Result<Vec<_>>>()
            .map(Manifests)
    }

    /// Get the manifest specified by the manifest path. Try to make an educated guess if no path is
    /// provided.
    pub fn get_local_one(manifest_path: &Option<PathBuf>) -> Result<Self> {
        let resolved_manifest_path: String = find(&manifest_path)?.to_string_lossy().into();

        let manifest = LocalManifest::find(&manifest_path)?;

        let mut cmd = cargo_metadata::MetadataCommand::new();
        cmd.no_deps();
        if let Some(path) = manifest_path {
            cmd.manifest_path(path);
        }
        let result = cmd.exec().chain_err(|| "Invalid manifest")?;
        let packages = result.packages;
        let package = packages
            .iter()
            .find(|p| p.manifest_path.to_string_lossy() == resolved_manifest_path)
            // If we have successfully got metadata, but our manifest path does not correspond to a
            // package, we must have been called against a virtual manifest.
            .chain_err(|| {
                "Found virtual manifest, but this command requires running against an \
                 actual package in this workspace. Try adding `--all`."
            })?;

        Ok(Manifests(vec![(manifest, package.to_owned())]))
    }
}
