//! Folders: a key that opens a page of further keys, like folders in the Stream Deck application.
//!
//! A folder's keys are kept in a profile of their own under `_folders/`, which the profile list hides. Its first key
//! is a back key that returns to the profile the folder was opened from. The folder key itself can show another
//! action (its only child, e.g. a volume display); a press always opens the folder instead of triggering that action.

use crate::shared::{ActionContext, ActionInstance, DeviceInfo, config_dir};
use crate::store::profiles::{ProfileStores, get_device_profiles};

use std::sync::LazyLock;

use dashmap::DashMap;
use tauri::{Emitter, Manager};

pub const FOLDER: &str = "opendeck.folder";
pub const BACK: &str = "opendeck.folderback";
/// Profiles holding the keys of folders, hidden from the profile list.
pub const PROFILE_PREFIX: &str = "_folders/";

/// Whether instances of the action hold child instances.
pub fn is_parent(uuid: &str) -> bool {
	matches!(uuid, "opendeck.multiaction" | "opendeck.toggleaction" | FOLDER)
}

/// The profile holding the keys of a folder instance.
pub fn folder_of(instance: &ActionInstance) -> Option<String> {
	if instance.action.uuid != FOLDER {
		return None;
	}
	instance.settings.get("folder")?.as_str().map(str::to_owned)
}

/// The profile each open folder was opened from, by device and folder profile, for its back key.
static OPENED_FROM: LazyLock<DashMap<(String, String), String>> = LazyLock::new(DashMap::new);

/// A profile id for a new folder that no other profile of the device uses.
fn new_folder_id(device: &str) -> Result<String, anyhow::Error> {
	let existing = get_device_profiles(device)?;
	let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
	// Profile ids end up in action contexts, which are separated by dots, so the id must not contain any.
	let mut n = millis;
	while existing.contains(&format!("{PROFILE_PREFIX}{n}")) {
		n += 1;
	}
	Ok(format!("{PROFILE_PREFIX}{n}"))
}

/// A back key for the first slot of a folder.
async fn back_key(device: &str, profile: &str) -> Option<ActionInstance> {
	let categories = crate::shared::CATEGORIES.read().await;
	let action = categories.values().flat_map(|c| c.actions.iter()).find(|a| a.uuid == BACK)?.clone();
	Some(ActionInstance {
		states: action.states.clone(),
		action,
		context: ActionContext {
			device: device.to_owned(),
			profile: profile.to_owned(),
			controller: "Keypad".to_owned(),
			position: 0,
			index: 0,
		},
		current_state: 0,
		settings: serde_json::json!({}),
		children: None,
	})
}

/// Creates the profile for a new folder, with a back key in its first slot, and returns its id.
pub async fn create(device: &DeviceInfo, stores: &mut ProfileStores) -> Result<String, anyhow::Error> {
	let id = new_folder_id(&device.id)?;
	let mut back = back_key(&device.id, &id).await;
	// New back keys look like one restyled before (e.g. with an icon from an icon pack), newest folder first.
	let mut folders: Vec<String> = get_device_profiles(&device.id)?.into_iter().filter(|p| p.starts_with(PROFILE_PREFIX)).collect();
	folders.sort_by(|a, b| b.cmp(a));
	for folder in folders {
		let store = stores.get_profile_store_mut(device, &folder).await?;
		let styled = store.value.keys.iter().flatten().find(|k| k.action.uuid == BACK && k.states.iter().any(|s| s.image_override.is_some()));
		if let (Some(styled), Some(back)) = (styled, back.as_mut()) {
			back.states = styled.states.clone();
			// Loaded images are files of the other folder, which may be deleted: give the copy its own.
			for state in &mut back.states {
				state.image_override = state.image_override.take().and_then(|image| embed(&image));
			}
			break;
		}
	}
	let store = stores.get_profile_store_mut(device, &id).await?;
	if let Some(slot) = store.value.keys.first_mut() {
		*slot = back;
	}
	store.save()?;
	Ok(id)
}

/// An image as a data URL: unchanged if it is one, else read from its file (None if that fails).
fn embed(image: &str) -> Option<String> {
	if image.starts_with("data:") {
		return Some(image.to_owned());
	}
	let path = std::path::Path::new(image);
	let mime = match path.extension()?.to_str()?.to_lowercase().as_str() {
		"svg" => "image/svg+xml".to_owned(),
		"jpg" => "image/jpeg".to_owned(),
		extension => format!("image/{extension}"),
	};
	use base64::Engine;
	Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(std::fs::read(path).ok()?)))
}

/// The folders whose keys are in `profile` (not including folders within those).
fn folders_in(device: &str, profile: &str) -> Vec<String> {
	#[cfg(target_os = "windows")]
	let profile = &profile.replace('/', "\\");
	let path = config_dir().join("profiles").join(device).join(format!("{profile}.json"));
	let Ok(json) = std::fs::read_to_string(path).map(|s| serde_json::from_str::<serde_json::Value>(&s)) else {
		return vec![];
	};
	let Ok(json) = json else { return vec![] };
	json.get("keys")
		.and_then(|keys| keys.as_array())
		.into_iter()
		.flatten()
		.filter(|key| key.pointer("/action/uuid").and_then(|v| v.as_str()) == Some(FOLDER))
		.filter_map(|key| key.pointer("/settings/folder").and_then(|v| v.as_str()).map(str::to_owned))
		.collect()
}

/// Deletes a folder's profile, and those of the folders inside it.
pub fn delete(device: &str, id: &str, stores: &mut ProfileStores) {
	let mut queue = vec![id.to_owned()];
	while let Some(id) = queue.pop() {
		// Only ever delete folder profiles, whatever a settings object claims.
		if !id.starts_with(PROFILE_PREFIX) {
			continue;
		}
		queue.extend(folders_in(device, &id));
		stores.delete_profile(device, &id);
		OPENED_FROM.retain(|(d, folder), _| !(d == device && *folder == id));
	}
}

/// Deletes the folders whose keys are in `profile`, before `profile` itself is deleted.
pub fn delete_folders_in(device: &str, profile: &str, stores: &mut ProfileStores) {
	for id in folders_in(device, profile) {
		delete(device, &id, stores);
	}
}

/// Copies a folder's profile, and those of the folders inside it, and returns the id of the copy.
pub async fn duplicate(device: &DeviceInfo, id: &str, stores: &mut ProfileStores) -> Result<String, anyhow::Error> {
	let copy = new_folder_id(&device.id)?;
	copy_profile(device, id, &copy, stores).await?;
	copy_folders_in(device, &copy, stores).await?;
	Ok(copy)
}

/// Gives the folder keys in `profile`, a copy of another profile, copies of their folders, so that the two profiles
/// don't share them.
pub async fn copy_folders_in(device: &DeviceInfo, profile: &str, stores: &mut ProfileStores) -> Result<(), anyhow::Error> {
	let mut queue = vec![profile.to_owned()];
	while let Some(profile) = queue.pop() {
		let nested: Vec<(usize, String)> = {
			let store = stores.get_profile_store_mut(device, &profile).await?;
			store.value.keys.iter().enumerate().filter_map(|(i, k)| Some((i, folder_of(k.as_ref()?)?))).collect()
		};
		for (position, folder) in nested {
			let nested_copy = new_folder_id(&device.id)?;
			copy_profile(device, &folder, &nested_copy, stores).await?;
			let store = stores.get_profile_store_mut(device, &profile).await?;
			if let Some(Some(key)) = store.value.keys.get_mut(position) {
				key.settings["folder"] = serde_json::Value::String(nested_copy.clone());
			}
			store.save()?;
			queue.push(nested_copy);
		}
	}
	Ok(())
}

async fn copy_profile(device: &DeviceInfo, from: &str, to: &str, stores: &mut ProfileStores) -> Result<(), anyhow::Error> {
	// The copy is made from the file, so write any changes that are still pending first.
	stores.get_profile_store_mut(device, from).await?.save()?;
	stores.rename_profile(device, from, to, true).await
}

fn switch_profile(device: &str, profile: &str) -> Result<(), anyhow::Error> {
	let app = crate::APP_HANDLE.get().unwrap();
	app.get_webview_window("main")
		.unwrap()
		.emit("switch_profile", serde_json::json!({ "device": device, "profile": profile }))?;
	Ok(())
}

/// Opens a folder on the device, from the profile its key is in.
pub fn open(device: &str, from: &str, folder: &str) -> Result<(), anyhow::Error> {
	if !get_device_profiles(device)?.iter().any(|p| p == folder) {
		return Err(anyhow::anyhow!("folder {folder} not found"));
	}
	OPENED_FROM.insert((device.to_owned(), folder.to_owned()), from.to_owned());
	switch_profile(device, folder)
}

/// Returns from the folder `profile` to the profile it was opened from.
pub fn back(device: &str, profile: &str) -> Result<(), anyhow::Error> {
	let profiles = get_device_profiles(device)?;
	let opened_from = OPENED_FROM.get(&(device.to_owned(), profile.to_owned())).map(|p| p.clone());
	// Without a record (e.g. after a restart), return to the profile containing the folder's key.
	let target = opened_from
		.filter(|p| profiles.contains(p))
		.or_else(|| profiles.iter().find(|p| folders_in(device, p).iter().any(|f| f == profile)).cloned())
		.or_else(|| profiles.iter().find(|p| !p.starts_with(PROFILE_PREFIX)).cloned());
	match target {
		Some(target) if target != profile => switch_profile(device, &target),
		_ => Ok(()),
	}
}
