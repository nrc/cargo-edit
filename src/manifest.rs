use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::{env, str};

use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};
use toml_edit;

use crate::dependency::Dependency;
use crate::errors::*;

const MANIFEST_FILENAME: &str = "Cargo.toml";

/// A Cargo manifest
#[derive(Debug, Clone)]
pub struct Manifest {
    /// Manifest contents as TOML data
    pub data: toml_edit::Document,
}

/// If a manifest is specified, return that one, otherwise perform a manifest search starting from
/// the current directory.
/// If a manifest is specified, return that one. If a path is specified, perform a manifest search
/// starting from there. If nothing is specified, start searching from the current directory
/// (`cwd`).
pub fn find(specified: &Option<PathBuf>) -> Result<PathBuf> {
    match *specified {
        Some(ref path)
            if fs::metadata(&path)
                .chain_err(|| "Failed to get cargo file metadata")?
                .is_file() =>
        {
            Ok(path.to_owned())
        }
        Some(ref path) => search(path),
        None => search(&env::current_dir().chain_err(|| "Failed to get current directory")?),
    }
}

/// Search for Cargo.toml in this directory and recursively up the tree until one is found.
fn search(dir: &Path) -> Result<PathBuf> {
    let manifest = dir.join(MANIFEST_FILENAME);

    if fs::metadata(&manifest).is_ok() {
        Ok(manifest)
    } else {
        dir.parent()
            .ok_or_else(|| ErrorKind::MissingManifest.into())
            .and_then(|dir| search(dir))
    }
}

fn merge_inline_table(old_dep: &mut toml_edit::Item, new: &toml_edit::Item) {
    for (k, v) in new
        .as_inline_table()
        .expect("expected an inline table")
        .iter()
    {
        old_dep[k] = toml_edit::value(v.clone());
    }
}

fn str_or_1_len_table(item: &toml_edit::Item) -> bool {
    item.is_str() || item.as_table_like().map(|t| t.len() == 1).unwrap_or(false)
}
/// Merge a new dependency into an old entry. See `Dependency::to_toml` for what the format of the
/// new dependency will be.
fn merge_dependencies(old_dep: &mut toml_edit::Item, new: &Dependency) {
    assert!(!old_dep.is_none());

    let new_toml = new.to_toml().1;

    if str_or_1_len_table(old_dep) {
        // The old dependency is just a version/git/path. We are safe to overwrite.
        *old_dep = new_toml;
    } else if old_dep.is_table_like() {
        for key in &["version", "path", "git"] {
            // remove this key/value pairs
            old_dep[key] = toml_edit::Item::None;
        }
        if let Some(name) = new_toml.as_str() {
            old_dep["version"] = toml_edit::value(name);
        } else {
            merge_inline_table(old_dep, &new_toml);
        }
    } else {
        unreachable!("Invalid old dependency type");
    }

    if let Some(t) = old_dep.as_inline_table_mut() {
        t.fmt()
    }
}

/// Print a message if the new dependency version is different from the old one.
fn print_upgrade_if_necessary(
    crate_name: &str,
    old_dep: &toml_edit::Item,
    new_version: &toml_edit::Item,
) -> Result<()> {
    let old_version = if str_or_1_len_table(old_dep) {
        old_dep.clone()
    } else if old_dep.is_table_like() {
        let version = old_dep["version"].clone();
        if version.is_none() {
            return Err("Missing version field".into());
        }
        version
    } else {
        unreachable!("Invalid old dependency type")
    };

    if let (Some(old_version), Some(new_version)) = (old_version.as_str(), new_version.as_str()) {
        if old_version == new_version {
            return Ok(());
        }
        let bufwtr = BufferWriter::stdout(ColorChoice::Always);
        let mut buffer = bufwtr.buffer();
        buffer
            .set_color(ColorSpec::new().set_fg(Some(Color::Green)).set_bold(true))
            .chain_err(|| "Failed to set output colour")?;
        write!(&mut buffer, "    Upgrading ").chain_err(|| "Failed to write upgrade message")?;
        buffer
            .set_color(&ColorSpec::new())
            .chain_err(|| "Failed to clear output colour")?;
        writeln!(
            &mut buffer,
            "{} v{} -> v{}",
            crate_name, old_version, new_version,
        )
        .chain_err(|| "Failed to write upgrade versions")?;
        bufwtr
            .print(&buffer)
            .chain_err(|| "Failed to print upgrade message")?;
    }
    Ok(())
}

impl Manifest {
    /// Look for a `Cargo.toml` file
    ///
    /// Starts at the given path an goes into its parent directories until the manifest file is
    /// found. If no path is given, the process's working directory is used as a starting point.
    pub fn find_file(path: &Option<PathBuf>) -> Result<File> {
        find(path).and_then(|path| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .chain_err(|| "Failed to find Cargo.toml")
        })
    }

    /// Open the `Cargo.toml` for a path (or the process' `cwd`)
    pub fn open(path: &Option<PathBuf>) -> Result<Manifest> {
        let mut file = Manifest::find_file(path)?;
        let mut data = String::new();
        file.read_to_string(&mut data)
            .chain_err(|| "Failed to read manifest contents")?;

        data.parse().chain_err(|| "Unable to parse Cargo.toml")
    }

    /// Get the specified table from the manifest.
    pub fn get_table<'a>(&'a mut self, table_path: &[String]) -> Result<&'a mut toml_edit::Item> {
        /// Descend into a manifest until the required table is found.
        fn descend<'a>(
            input: &'a mut toml_edit::Item,
            path: &[String],
        ) -> Result<&'a mut toml_edit::Item> {
            if let Some(segment) = path.get(0) {
                let value = input[&segment].or_insert(toml_edit::table());

                if value.is_table_like() {
                    descend(value, &path[1..])
                } else {
                    Err(ErrorKind::NonExistentTable(segment.clone()).into())
                }
            } else {
                Ok(input)
            }
        }

        descend(&mut self.data.root, table_path)
    }

    /// Get all sections in the manifest that exist and might contain dependencies.
    /// The returned items are always `Table` or `InlineTable`.
    pub fn get_sections(&self) -> Vec<(Vec<String>, toml_edit::Item)> {
        let mut sections = Vec::new();

        for dependency_type in &["dev-dependencies", "build-dependencies", "dependencies"] {
            // Dependencies can be in the three standard sections...
            if self.data[dependency_type].is_table_like() {
                sections.push((
                    vec![String::from(*dependency_type)],
                    self.data[dependency_type].clone(),
                ))
            }

            // ... and in `target.<target>.(build-/dev-)dependencies`.
            let target_sections = self
                .data
                .as_table()
                .get("target")
                .and_then(toml_edit::Item::as_table_like)
                .into_iter()
                .flat_map(toml_edit::TableLike::iter)
                .filter_map(|(target_name, target_table)| {
                    let dependency_table = &target_table[dependency_type];
                    dependency_table.as_table_like().map(|_| {
                        (
                            vec![
                                "target".to_string(),
                                target_name.to_string(),
                                String::from(*dependency_type),
                            ],
                            dependency_table.clone(),
                        )
                    })
                });

            sections.extend(target_sections);
        }

        sections
    }

    /// Overwrite a file with TOML data.
    pub fn write_to_file(&self, file: &mut File) -> Result<()> {
        if self.data["package"].is_none() && self.data["project"].is_none() {
            if !self.data["workspace"].is_none() {
                return Err(ErrorKind::UnexpectedRootManifest.into());
            } else {
                return Err(ErrorKind::InvalidManifest.into());
            }
        }

        let s = self.data.to_string_in_original_order();
        let new_contents_bytes = s.as_bytes();

        // We need to truncate the file, otherwise the new contents
        // will be mixed up with the old ones.
        file.set_len(new_contents_bytes.len() as u64)
            .chain_err(|| "Failed to truncate Cargo.toml")?;
        file.write_all(new_contents_bytes)
            .chain_err(|| "Failed to write updated Cargo.toml")
    }

    /// Add entry to a Cargo.toml.
    pub fn insert_into_table(&mut self, table_path: &[String], dep: &Dependency) -> Result<()> {
        let table = self.get_table(table_path)?;

        if table[&dep.name].is_none() {
            // insert a new entry
            let (ref name, ref mut new_dependency) = dep.to_toml();
            table[name] = new_dependency.clone();
        } else {
            // update an existing entry

            // if the `dep` is renamed in the `add` command,
            // but was present before, then we need to remove
            // the old entry and insert a new one
            // as the key has changed, e.g. from
            // a = "0.1"
            // to
            // alias = { version = "0.2", package = "a" }
            if let Some(renamed) = dep.rename() {
                let old_copy = table[&dep.name].clone();
                table[renamed] = old_copy;
                table[&dep.name] = toml_edit::Item::None;
            }
            merge_dependencies(&mut table[dep.name_in_manifest()], dep);
            if let Some(t) = table.as_inline_table_mut() {
                t.fmt()
            }
        }
        Ok(())
    }

    /// Update an entry in Cargo.toml.
    pub fn update_table_entry(
        &mut self,
        table_path: &[String],
        dep: &Dependency,
        dry_run: bool,
    ) -> Result<()> {
        self.update_table_named_entry(table_path, dep.name_in_manifest(), dep, dry_run)
    }

    /// Update an entry with a specified name in Cargo.toml.
    pub fn update_table_named_entry(
        &mut self,
        table_path: &[String],
        item_name: &str,
        dep: &Dependency,
        dry_run: bool,
    ) -> Result<()> {
        let table = self.get_table(table_path)?;
        let new_dep = dep.to_toml().1;

        // If (and only if) there is an old entry, merge the new one in.
        if !table[item_name].is_none() {
            if let Err(e) = print_upgrade_if_necessary(&dep.name, &table[item_name], &new_dep) {
                eprintln!("Error while displaying upgrade message, {}", e);
            }
            if !dry_run {
                merge_dependencies(&mut table[item_name], dep);
                if let Some(t) = table.as_inline_table_mut() {
                    t.fmt()
                }
            }
        }

        Ok(())
    }

    /// Remove entry from a Cargo.toml.
    ///
    /// # Examples
    ///
    /// ```
    ///   use cargo_edit::{Dependency, Manifest};
    ///   use toml_edit;
    ///
    ///   let mut manifest = Manifest { data: toml_edit::Document::new() };
    ///   let dep = Dependency::new("cargo-edit").set_version("0.1.0");
    ///   let _ = manifest.insert_into_table(&vec!["dependencies".to_owned()], &dep);
    ///   assert!(manifest.remove_from_table("dependencies", &dep.name).is_ok());
    ///   assert!(manifest.remove_from_table("dependencies", &dep.name).is_err());
    ///   assert!(manifest.data["dependencies"].is_none());
    /// ```
    pub fn remove_from_table(&mut self, table: &str, name: &str) -> Result<()> {
        if !self.data[table].is_table_like() {
            return Err(ErrorKind::NonExistentTable(table.into()).into());
        } else {
            {
                let dep = &mut self.data[table][name];
                if dep.is_none() {
                    return Err(ErrorKind::NonExistentDependency(name.into(), table.into()).into());
                }
                // remove the dependency
                *dep = toml_edit::Item::None;
            }

            // remove table if empty
            if self.data[table].as_table_like().unwrap().is_empty() {
                self.data[table] = toml_edit::Item::None;
            }
        }
        Ok(())
    }

    /// Add multiple dependencies to manifest
    pub fn add_deps(&mut self, table: &[String], deps: &[Dependency]) -> Result<()> {
        deps.iter()
            .map(|dep| self.insert_into_table(table, dep))
            .collect::<Result<Vec<_>>>()?;

        Ok(())
    }

    /// Sort a table using its natural order.
    ///
    /// Returns an error if the table cannot be found.
    pub fn sort_table(&mut self, table_path: &[String]) -> Result<()> {
        if let Some(table) = self.get_table(table_path)?.as_table_mut() {
            table.sort_values();
        }
        Ok(())
    }
}

impl str::FromStr for Manifest {
    type Err = Error;

    /// Read manifest data from string
    fn from_str(input: &str) -> ::std::result::Result<Self, Self::Err> {
        let d: toml_edit::Document = input.parse().chain_err(|| "Manifest not valid TOML")?;

        Ok(Manifest { data: d })
    }
}

/// A Cargo manifest that is available locally.
#[derive(Debug)]
pub struct LocalManifest {
    /// Path to the manifest
    pub path: PathBuf,
    /// Manifest contents
    manifest: Manifest,
}

impl Deref for LocalManifest {
    type Target = Manifest;

    fn deref(&self) -> &Manifest {
        &self.manifest
    }
}

impl DerefMut for LocalManifest {
    fn deref_mut(&mut self) -> &mut Manifest {
        &mut self.manifest
    }
}

impl LocalManifest {
    /// Construct a `LocalManifest`. If no path is provided, make an educated guess as to which one
    /// the user means.
    pub fn find(path: &Option<PathBuf>) -> Result<Self> {
        let path = find(path)?;
        Self::try_new(&path)
    }

    /// Construct the `LocalManifest` corresponding to the `Path` provided.
    pub fn try_new(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        Ok(LocalManifest {
            manifest: Manifest::open(&Some(path.clone()))?,
            path,
        })
    }

    /// Get the `File` corresponding to this manifest.
    fn get_file(&self) -> Result<File> {
        Manifest::find_file(&Some(self.path.clone()))
    }

    /// Instruct this manifest to upgrade a single dependency. If this manifest does not have that
    /// dependency, it does nothing.
    pub fn upgrade(&mut self, dependency: &Dependency, dry_run: bool) -> Result<()> {
        for (table_path, table) in self.get_sections() {
            let table_like = table.as_table_like().expect("Unexpected non-table");
            for (name, toml_item) in table_like.iter() {
                let dep_name = toml_item
                    .as_table_like()
                    .and_then(|t| t.get("package").and_then(|p| p.as_str()))
                    .unwrap_or(name);
                if dep_name == dependency.name {
                    self.manifest.update_table_named_entry(
                        &table_path,
                        &name,
                        dependency,
                        dry_run,
                    )?;
                }
            }
        }

        let mut file = self.get_file()?;
        self.write_to_file(&mut file)
            .chain_err(|| "Failed to write new manifest contents")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependency::Dependency;
    use toml_edit;

    #[test]
    fn add_remove_dependency() {
        let mut manifest = Manifest {
            data: toml_edit::Document::new(),
        };
        let clone = manifest.clone();
        let dep = Dependency::new("cargo-edit").set_version("0.1.0");
        let _ = manifest.insert_into_table(&["dependencies".to_owned()], &dep);
        assert!(manifest
            .remove_from_table("dependencies", &dep.name)
            .is_ok());
        assert_eq!(manifest.data.to_string(), clone.data.to_string());
    }

    #[test]
    fn update_dependency() {
        let mut manifest = Manifest {
            data: toml_edit::Document::new(),
        };
        let dep = Dependency::new("cargo-edit").set_version("0.1.0");
        manifest
            .insert_into_table(&["dependencies".to_owned()], &dep)
            .unwrap();

        let new_dep = Dependency::new("cargo-edit").set_version("0.2.0");
        manifest
            .update_table_entry(&["dependencies".to_owned()], &new_dep, false)
            .unwrap();
    }

    #[test]
    fn update_wrong_dependency() {
        let mut manifest = Manifest {
            data: toml_edit::Document::new(),
        };
        let dep = Dependency::new("cargo-edit").set_version("0.1.0");
        manifest
            .insert_into_table(&["dependencies".to_owned()], &dep)
            .unwrap();
        let original = manifest.clone();

        let new_dep = Dependency::new("wrong-dep").set_version("0.2.0");
        manifest
            .update_table_entry(&["dependencies".to_owned()], &new_dep, false)
            .unwrap();

        assert_eq!(manifest.data.to_string(), original.data.to_string());
    }

    #[test]
    fn remove_dependency_no_section() {
        let mut manifest = Manifest {
            data: toml_edit::Document::new(),
        };
        let dep = Dependency::new("cargo-edit").set_version("0.1.0");
        assert!(manifest
            .remove_from_table("dependencies", &dep.name)
            .is_err());
    }

    #[test]
    fn remove_dependency_non_existent() {
        let mut manifest = Manifest {
            data: toml_edit::Document::new(),
        };
        let dep = Dependency::new("cargo-edit").set_version("0.1.0");
        let other_dep = Dependency::new("other-dep").set_version("0.1.0");
        let _ = manifest.insert_into_table(&["dependencies".to_owned()], &other_dep);
        assert!(manifest
            .remove_from_table("dependencies", &dep.name)
            .is_err());
    }
}
