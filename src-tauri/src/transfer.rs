//! Portable preferences and checksum-verified imports. Never imports credentials
//! or executable output actions from a configuration bundle.
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};

use crate::context::AppContext;
use crate::profile::manifest::{all_manifests, ModelManifest, RuntimeKind};
use crate::settings::{AppSettings, DictionaryTerm, Mode, Snippet};

const BUNDLE_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
// Machine placement, network/API privileges, encryption, devices, paths and
// output commands deliberately stay local, including in replace mode.
const PORTABLE_KEYS: &[&str] = &[
    "language",
    "intelligence_tier",
    "cleanup_level",
    "style",
    "auto_style_from_app",
    "dictation_mode",
    "hotkey",
    "hotkeys",
    "hotkey_mode",
    "default_mode_id",
    "mode_triggers_enabled",
    "spoken_punctuation_enabled",
    "filler_removal_enabled",
    "clipboard_restore_enabled",
    "overlay_position",
    "overlay_theme",
    "app_theme",
    "accent_color",
    "hud_scale",
    "waveform_style",
    "reduce_motion",
    "ui_font_scale",
    "voice_commands_enabled",
    "send_key",
    "excluded_apps",
    "hud_contrast",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBundle {
    pub version: u32,
    pub settings_version: u32,
    pub settings: Value,
    pub snippets: Vec<Snippet>,
    pub modes: Vec<Mode>,
    pub dictionary_terms: Vec<DictionaryTerm>,
    pub custom_replacements: Vec<crate::formatting::replacements::ReplacementRule>,
}

fn safe_action(mode: &Mode) -> bool {
    !matches!(
        mode.output,
        crate::settings::OutputAction::RunCommand { .. }
            | crate::settings::OutputAction::AppendFile { .. }
    )
}

pub fn make_bundle(settings: &AppSettings) -> Result<ConfigBundle, String> {
    let all = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    let mut portable: Map<String, Value> = PORTABLE_KEYS
        .iter()
        .filter_map(|key| {
            all.get(*key)
                .map(|value| ((*key).to_owned(), value.clone()))
        })
        .collect();
    if let Some(apps) = portable
        .get_mut("excluded_apps")
        .and_then(Value::as_array_mut)
    {
        for app in apps {
            if let Some(process) = app.as_str() {
                *app = Value::String(crate::settings::expansion::process_basename(process));
            }
        }
    }
    let mut modes = settings.modes.clone();
    for mode in &mut modes {
        mode.triggers.apps = mode
            .triggers
            .apps
            .iter()
            .map(|process| crate::settings::expansion::process_basename(process))
            .collect();
        if !safe_action(mode) {
            mode.output = crate::settings::OutputAction::Paste;
        }
    }
    Ok(ConfigBundle {
        version: BUNDLE_VERSION,
        settings_version: settings.settings_version,
        settings: Value::Object(portable),
        snippets: settings.snippets.clone(),
        modes,
        dictionary_terms: settings.dictionary_terms.clone(),
        custom_replacements: settings.custom_replacements.clone(),
    })
}

fn merge_by_id<T: Clone>(existing: &[T], incoming: &[T], id: impl Fn(&T) -> &str) -> Vec<T> {
    let mut result = existing.to_vec();
    for item in incoming {
        if let Some(index) = result.iter().position(|old| id(old) == id(item)) {
            result[index] = item.clone();
        } else {
            result.push(item.clone());
        }
    }
    result
}

pub fn bundle_patch(
    current: &AppSettings,
    bundle: &ConfigBundle,
    replace: bool,
) -> Result<Value, String> {
    if bundle.version != BUNDLE_VERSION
        || bundle.settings_version != crate::settings::config::CURRENT_SETTINGS_VERSION
    {
        return Err(
            "Unsupported configuration version. Export from the same Reflow settings version."
                .into(),
        );
    }
    if bundle.modes.iter().any(|mode| !safe_action(mode)) {
        return Err("Configuration contains a shell command or file output path. Configure those actions locally after import.".into());
    }
    let incoming = bundle
        .settings
        .as_object()
        .ok_or("Bundle settings must be an object")?;
    if incoming
        .keys()
        .any(|key| !PORTABLE_KEYS.contains(&key.as_str()))
    {
        return Err("Configuration contains machine-specific or privileged settings.".into());
    }
    if bundle.modes.len() > 200
        || bundle.snippets.len() > 2_000
        || bundle.dictionary_terms.len() > 10_000
        || bundle.custom_replacements.len() > 10_000
    {
        return Err("Configuration bundle is too large.".into());
    }
    if bundle.dictionary_terms.iter().any(|term| {
        term.id.len() > 256
            || term.term.trim().is_empty()
            || term.term.len() > 512
            || term.preferred_spelling.len() > 512
            || term.category.len() > 256
    }) || bundle.custom_replacements.iter().any(|rule| {
        rule.id.len() > 256
            || rule.before.trim().is_empty()
            || rule.before.len() > 4000
            || rule.after.len() > 4000
    }) {
        return Err("Configuration contains an invalid dictionary term or replacement.".into());
    }
    let mut portable_modes = bundle.modes.clone();
    for mode in &mut portable_modes {
        // Imported activation, triggers or instructions must not acquire a
        // trusted local command/path. Preserve that whole local mode.
        if let Some(local) = current.modes.iter().find(|local| local.id == mode.id) {
            if !safe_action(local) {
                *mode = local.clone();
            }
        }
    }
    let mut patch = incoming.clone();
    let (modes, snippets, terms, replacements) = if replace {
        (
            portable_modes.clone(),
            bundle.snippets.clone(),
            bundle.dictionary_terms.clone(),
            bundle.custom_replacements.clone(),
        )
    } else {
        let mut replacements = current.custom_replacements.clone();
        for rule in &bundle.custom_replacements {
            if let Some(index) = replacements
                .iter()
                .position(|old| old.before == rule.before)
            {
                replacements[index] = rule.clone();
            } else {
                replacements.push(rule.clone());
            }
        }
        (
            merge_by_id(&current.modes, &portable_modes, |mode| &mode.id),
            merge_by_id(&current.snippets, &bundle.snippets, |snippet| &snippet.id),
            merge_by_id(
                &current.dictionary_terms,
                &bundle.dictionary_terms,
                |term| &term.id,
            ),
            replacements,
        )
    };
    if modes.iter().any(|mode| !safe_action(mode)) {
        // These global selectors can activate a retained privileged mode even
        // when its own triggers/enabled flag are unchanged.
        patch.insert(
            "default_mode_id".into(),
            current.default_mode_id.clone().into(),
        );
        patch.insert(
            "mode_triggers_enabled".into(),
            current.mode_triggers_enabled.into(),
        );
    }
    patch.insert(
        "modes".into(),
        serde_json::to_value(modes).map_err(|e| e.to_string())?,
    );
    patch.insert(
        "snippets".into(),
        serde_json::to_value(snippets).map_err(|e| e.to_string())?,
    );
    patch.insert(
        "dictionary_terms".into(),
        serde_json::to_value(terms).map_err(|e| e.to_string())?,
    );
    patch.insert(
        "custom_replacements".into(),
        serde_json::to_value(replacements).map_err(|e| e.to_string())?,
    );
    // Deserialize + validate before touching the live settings file.
    let mut candidate = serde_json::to_value(current).map_err(|e| e.to_string())?;
    candidate
        .as_object_mut()
        .ok_or("Invalid settings")?
        .extend(patch.clone());
    let candidate: AppSettings = serde_json::from_value(candidate).map_err(|e| e.to_string())?;
    candidate.validate_expansion()?;
    Ok(Value::Object(patch))
}

#[tauri::command]
pub fn export_config(path: String, ctx: State<'_, AppContext>) -> Result<String, String> {
    let destination = PathBuf::from(path);
    let bytes = serde_json::to_vec_pretty(&make_bundle(&ctx.settings_store.get())?)
        .map_err(|e| e.to_string())?;
    // Explicit user-selected output; create_new avoids overwriting another file.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|e| format!("Could not create config export: {e}"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn import_config(
    path: String,
    replace: Option<bool>,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<AppSettings, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_CONFIG_BYTES {
        return Err("Configuration is larger than 4 MB.".into());
    }
    let bundle: ConfigBundle = serde_json::from_reader(file.take(MAX_CONFIG_BYTES))
        .map_err(|e| format!("Invalid configuration bundle: {e}"))?;
    let patch = bundle_patch(&ctx.settings_store.get(), &bundle, replace.unwrap_or(false))?;
    // Use the normal update path: validation, persistence, hotkeys, runtime and
    // retention effects stay consistent with edits made in Settings.
    crate::commands::update_settings(patch, app, ctx)
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportedModel {
    pub id: String,
    pub path: String,
    pub bytes: u64,
}

fn digest(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn verified_copy(source: &Path, destination: &Path, sha256: &str) -> Result<u64, String> {
    if std::fs::symlink_metadata(source)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Model imports cannot contain symbolic links.".into());
    }
    if digest(source)? != sha256 {
        return Err("Model SHA-256 does not match the pinned manifest.".into());
    }
    let copied = std::fs::copy(source, destination).map_err(|e| e.to_string())?;
    if digest(destination)? != sha256 {
        return Err("Copied model failed SHA-256 verification.".into());
    }
    Ok(copied)
}

/// Pure disk import; no runtime launch or network. Stage all files before install.
pub fn import_model_into(source: &Path, models_dir: &Path) -> Result<ImportedModel, String> {
    if !source.exists() {
        return Err("Model path does not exist.".into());
    }
    let weight = if source.is_file() {
        source.to_owned()
    } else {
        all_manifests()
            .find_map(|manifest| {
                source
                    .join(manifest.filename)
                    .is_file()
                    .then(|| source.join(manifest.filename))
            })
            .ok_or("Directory contains no supported manifest weight file.")?
    };
    let actual = digest(&weight)?;
    let manifest: &ModelManifest = all_manifests().find(|manifest| manifest.sha256 == actual)
        .ok_or("Model SHA-256 is not recognized by the pinned manifests. Arbitrary models cannot be imported.")?;
    let source_dir = weight.parent().ok_or("Model path has no parent")?;
    if source.is_file() && manifest.runtime == RuntimeKind::PythonAsr {
        return Err("Python ASR requires the full model directory, including tokenizer and processor JSON files.".into());
    }
    if manifest.runtime == RuntimeKind::PythonAsr && !source_dir.join("config.json").is_file() {
        return Err("Python model directory is missing config.json.".into());
    }
    let target_directory = models_dir.join(manifest.dir_name);
    let destination = target_directory.join(manifest.filename);
    if destination.exists() {
        return Err(
            "This model is already installed. Remove it before importing a replacement.".into(),
        );
    }
    if manifest.runtime != RuntimeKind::LlamaServer && target_directory.exists() {
        return Err(
            "Model directory already exists. Remove the incomplete installation before importing."
                .into(),
        );
    }
    std::fs::create_dir_all(models_dir).map_err(|e| e.to_string())?;
    let stage = models_dir.join(format!(".import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut bytes = verified_copy(&weight, &stage.join(manifest.filename), manifest.sha256)?;
        for auxiliary in manifest.auxiliary_files {
            bytes += verified_copy(
                &source_dir.join(auxiliary.filename),
                &stage.join(auxiliary.filename),
                auxiliary.sha256,
            )
            .map_err(|e| format!("Required companion {}: {e}", auxiliary.filename))?;
        }
        if manifest.runtime == RuntimeKind::PythonAsr {
            for entry in std::fs::read_dir(source_dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let metadata = entry.file_type().map_err(|e| e.to_string())?;
                let path = entry.path();
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                if !matches!(extension, "json" | "txt" | "jinja" | "tiktoken") {
                    continue;
                }
                if metadata.is_symlink() || !metadata.is_file() {
                    return Err("Model metadata must be regular files.".into());
                }
                if entry.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
                    return Err("Model metadata file is larger than 64 MB.".into());
                }
                if extension == "json" {
                    let value: Value = serde_json::from_reader(
                        std::fs::File::open(&path).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    if value.get("auto_map").is_some() {
                        return Err("Model metadata requiring remote code is not supported.".into());
                    }
                }
                bytes += std::fs::copy(&path, stage.join(entry.file_name()))
                    .map_err(|e| e.to_string())?;
            }
        }
        let target_directory = models_dir.join(manifest.dir_name);
        let destination = target_directory.join(manifest.filename);
        // Import does not overwrite installed weights or remove a user's model.
        if destination.exists() {
            return Err(
                "This model is already installed. Remove it before importing a replacement.".into(),
            );
        }
        if manifest.runtime == RuntimeKind::LlamaServer {
            std::fs::create_dir_all(&target_directory).map_err(|e| e.to_string())?;
            std::fs::rename(stage.join(manifest.filename), &destination)
                .map_err(|e| e.to_string())?;
        } else {
            if target_directory.exists() {
                return Err("Model directory already exists. Remove the incomplete installation before importing.".into());
            }
            std::fs::rename(&stage, &target_directory).map_err(|e| e.to_string())?;
        }
        Ok(ImportedModel {
            id: manifest.id.into(),
            path: destination.to_string_lossy().into_owned(),
            bytes,
        })
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(stage);
    }
    result
}

#[tauri::command]
pub async fn import_model_file(
    path: String,
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<ImportedModel, String> {
    let _operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Finish the current operation before importing a model.")?;
    let imported = tauri::async_runtime::spawn_blocking(move || {
        import_model_into(
            Path::new(&path),
            &crate::platform::PlatformSys::get_app_dir().join("models"),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    let _ = app.emit("models:changed", &imported);
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_copy_rejects_corruption_before_copying_and_checks_destination() {
        let directory =
            std::env::temp_dir().join(format!("reflow-verified-copy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("source");
        let destination = directory.join("destination");
        std::fs::write(&source, b"weights").unwrap();
        let checksum = digest(&source).unwrap();
        assert_eq!(verified_copy(&source, &destination, &checksum).unwrap(), 7);
        assert_eq!(digest(&destination).unwrap(), checksum);
        std::fs::remove_file(&destination).unwrap();
        std::fs::write(&source, b"corruption").unwrap();
        assert!(verified_copy(&source, &destination, &checksum)
            .unwrap_err()
            .contains("SHA-256"));
        assert!(!destination.exists());
        std::fs::remove_file(source).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
