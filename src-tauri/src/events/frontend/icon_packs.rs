//! Icon packs in the Stream Deck format, installed into `icon_packs` in the config directory.
//!
//! A pack is a folder `<id>.sdIconPack` holding `manifest.json` (Name, Author, Version, Icon, ...), `icons.json`
//! (a list of `{ "path", "name", "tags" }`) and the icons themselves in `icons/`. Packs are distributed as
//! `.streamDeckIconPack` files, zip archives containing that folder.

use super::Error;
use crate::shared::config_dir;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::command;

const SUFFIX: &str = ".sdIconPack";

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Manifest {
	name: String,
	#[serde(default)]
	author: String,
	#[serde(default)]
	version: String,
	#[serde(default)]
	description: String,
	#[serde(default)]
	icon: String,
}

#[derive(Deserialize)]
struct IconEntry {
	path: String,
	#[serde(default)]
	name: String,
	#[serde(default)]
	tags: Vec<String>,
}

#[derive(Serialize)]
pub struct Icon {
	/// Absolute path of the image.
	path: String,
	name: String,
	tags: Vec<String>,
}

#[derive(Serialize)]
pub struct IconPack {
	id: String,
	name: String,
	author: String,
	version: String,
	description: String,
	/// Absolute path of the pack's own image, if it has one.
	icon: Option<String>,
	icons: Vec<Icon>,
}

fn packs_dir() -> PathBuf {
	config_dir().join("icon_packs")
}

/// `path` inside `root`, or None if it would leave it (e.g. through "..").
fn within(root: &Path, path: &str) -> Option<PathBuf> {
	let root = root.canonicalize().ok()?;
	let full = root.join(path).canonicalize().ok()?;
	full.starts_with(&root).then_some(full)
}

fn read_pack(dir: &Path) -> Result<IconPack, anyhow::Error> {
	let manifest: Manifest = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json"))?)?;
	let entries: Vec<IconEntry> = serde_json::from_str(&std::fs::read_to_string(dir.join("icons.json"))?)?;
	let icons_dir = dir.join("icons");
	let icons = entries
		.into_iter()
		.filter_map(|entry| {
			let path = within(&icons_dir, &entry.path)?;
			let name = if entry.name.is_empty() { path.file_stem()?.to_string_lossy().into_owned() } else { entry.name };
			Some(Icon {
				path: path.to_string_lossy().into_owned(),
				name,
				tags: entry.tags,
			})
		})
		.collect();
	let icon = (!manifest.icon.is_empty()).then(|| within(dir, &manifest.icon)).flatten().map(|p| p.to_string_lossy().into_owned());
	Ok(IconPack {
		id: dir.file_name().unwrap_or_default().to_string_lossy().into_owned(),
		name: manifest.name,
		author: manifest.author,
		version: manifest.version,
		description: manifest.description,
		icon,
		icons,
	})
}

/// The installed icon packs, by name.
#[command]
pub async fn list_icon_packs() -> Result<Vec<IconPack>, Error> {
	let mut packs = vec![];
	let Ok(entries) = std::fs::read_dir(packs_dir()) else { return Ok(packs) };
	for entry in entries.flatten() {
		let path = entry.path();
		if !path.is_dir() || !path.to_string_lossy().ends_with(SUFFIX) {
			continue;
		}
		match read_pack(&path) {
			Ok(pack) => packs.push(pack),
			Err(error) => log::warn!("Failed to read icon pack {}: {error}", path.display()),
		}
	}
	packs.sort_by_key(|p| p.name.to_lowercase());
	Ok(packs)
}

/// Installs an icon pack from a `.streamDeckIconPack` file (or a zip of an `.sdIconPack` folder), or from such a
/// folder, replacing an installed version. Returns the pack's id.
#[command]
pub async fn install_icon_pack(path: String) -> Result<String, Error> {
	let source = PathBuf::from(&path);
	let target_dir = packs_dir();
	std::fs::create_dir_all(&target_dir).map_err(anyhow::Error::from)?;

	if source.is_dir() {
		let id = source.file_name().unwrap_or_default().to_string_lossy().into_owned();
		if !id.ends_with(SUFFIX) {
			return Err(Error::new(format!("{path} is not an {SUFFIX} folder")));
		}
		read_pack(&source)?;
		let target = target_dir.join(&id);
		let _ = std::fs::remove_dir_all(&target);
		crate::shared::copy_dir(&source, &target).map_err(anyhow::Error::from)?;
		return Ok(id);
	}

	let bytes = std::fs::read(&source).map_err(anyhow::Error::from)?;
	let id = crate::zip_extract::dir_name_with_suffix(std::io::Cursor::new(&bytes), &SUFFIX.to_lowercase()).map_err(|_| Error::new(format!("{path} contains no {SUFFIX} folder")))?;
	// Extract next to the installed packs first, so that a broken archive leaves the installed version alone.
	let temp = config_dir().join("temp").join("icon_pack");
	let _ = std::fs::remove_dir_all(&temp);
	std::fs::create_dir_all(&temp).map_err(anyhow::Error::from)?;
	crate::zip_extract::extract(std::io::Cursor::new(bytes), &temp).map_err(|e| Error::new(format!("Failed to unzip {path}: {e}")))?;
	let extracted = temp.join(&id);
	if let Err(error) = read_pack(&extracted) {
		let _ = std::fs::remove_dir_all(&temp);
		return Err(error.into());
	}
	let target = target_dir.join(&id);
	let _ = std::fs::remove_dir_all(&target);
	std::fs::rename(&extracted, &target).map_err(anyhow::Error::from)?;
	let _ = std::fs::remove_dir_all(&temp);
	Ok(id)
}

/// Removes an installed icon pack. Keys keep icons chosen from it, as they store their own copy.
#[command]
pub async fn remove_icon_pack(id: String) -> Result<(), Error> {
	if !id.ends_with(SUFFIX) || id.contains(['/', '\\']) || id.starts_with('.') {
		return Err(Error::new(format!("invalid icon pack {id}")));
	}
	std::fs::remove_dir_all(packs_dir().join(id)).map_err(anyhow::Error::from)?;
	Ok(())
}
