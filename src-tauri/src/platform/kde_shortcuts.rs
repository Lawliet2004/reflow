//! KDE's configuration API edits the portal-owned action in place. The portal
//! still delivers press/release events; no second global-key listener is used.
use std::time::Duration;

use tauri_plugin_global_shortcut::{Modifiers, Shortcut};
use zbus::Proxy;

type KeySequence = (Vec<i32>,);

fn key_sequence(key: i32) -> KeySequence {
    // Qt's QKeySequence wire contract always has four entries. Older KDE
    // decoders read all four without checking the array length.
    (vec![key, 0, 0, 0],)
}

pub fn supported_desktop() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|name| name.eq_ignore_ascii_case("kde") || name.eq_ignore_ascii_case("plasma"))
}

/// Qt's public key/modifier encoding, used by KGlobalAccel's D-Bus API.
/// See qtbase/src/corelib/global/qnamespace.h. This translates key names,
/// rather than assuming a keycode, keyboard layout, or user binding.
fn qt_key(shortcut: &str) -> Result<i32, String> {
    if shortcut.trim().is_empty() {
        return Err("Choose a key combination first.".into());
    }
    let parsed: Shortcut = crate::hotkey::HotkeyManager::normalize_shortcut(shortcut)
        .parse()
        .map_err(|error| format!("Invalid shortcut: {error}"))?;
    let name = parsed.key.to_string();
    let key = if let Some(letter) = name.strip_prefix("Key").filter(|s| s.len() == 1) {
        i32::from(letter.as_bytes()[0])
    } else if let Some(digit) = name.strip_prefix("Digit").filter(|s| s.len() == 1) {
        i32::from(digit.as_bytes()[0])
    } else if let Some(number) = name.strip_prefix('F').and_then(|s| s.parse::<i32>().ok()) {
        if !(1..=35).contains(&number) {
            return Err("This function key cannot be configured here.".into());
        }
        0x01000030 + number - 1
    } else {
        match name.as_str() {
            "Space" => 0x20,
            "Escape" => 0x01000000,
            "Tab" => 0x01000001,
            "Backspace" => 0x01000003,
            "Enter" => 0x01000004,
            "Insert" => 0x01000006,
            "Delete" => 0x01000007,
            "Pause" => 0x01000008,
            "PrintScreen" => 0x01000009,
            "Home" => 0x01000010,
            "End" => 0x01000011,
            "ArrowLeft" => 0x01000012,
            "ArrowUp" => 0x01000013,
            "ArrowRight" => 0x01000014,
            "ArrowDown" => 0x01000015,
            "PageUp" => 0x01000016,
            "PageDown" => 0x01000017,
            "CapsLock" => 0x01000024,
            "NumLock" => 0x01000025,
            "ScrollLock" => 0x01000026,
            "ContextMenu" => 0x01000055,
            "Backquote" => b'`'.into(),
            "Minus" => b'-'.into(),
            "Equal" => b'='.into(),
            "BracketLeft" => b'['.into(),
            "BracketRight" => b']'.into(),
            "Backslash" | "IntlBackslash" => b'\\'.into(),
            "Semicolon" => b';'.into(),
            "Quote" => b'\''.into(),
            "Comma" => b','.into(),
            "Period" => b'.'.into(),
            "Slash" => b'/'.into(),
            _ => return Err("Choose this key using Configure desktop shortcuts instead.".into()),
        }
    };
    let mut value = key;
    for (modifier, qt) in [
        (Modifiers::SHIFT, 0x02000000),
        (Modifiers::CONTROL, 0x04000000),
        (Modifiers::ALT, 0x08000000),
        (Modifiers::SUPER, 0x10000000),
    ] {
        if parsed.mods.contains(modifier) {
            value |= qt;
        }
    }
    Ok(value)
}

async fn assign(
    proxy: &Proxy<'_>,
    app_id: &str,
    action: &str,
    key: Option<i32>,
) -> Result<(), String> {
    let actions: Vec<Vec<String>> = proxy
        .call("allActionsForComponent", &(vec![app_id],))
        .await
        .map_err(|error| error.to_string())?;
    let target = actions
        .iter()
        .find(|id| {
            id.first().is_some_and(|id| id == app_id) && id.get(1).is_some_and(|id| id == action)
        })
        .ok_or("This shortcut is not registered yet. Allow Reflow's shortcuts, then try again.")?;
    let previous: Vec<KeySequence> = proxy
        .call("shortcutKeys", &(target,))
        .await
        .map_err(|error| error.to_string())?;
    let requested: Vec<KeySequence> = key.map(|key| vec![key_sequence(key)]).unwrap_or_default();
    if previous == requested {
        return Ok(());
    }
    if let Some(key) = key {
        for id in actions.iter().filter(|id| *id != target) {
            let keys: Vec<KeySequence> = proxy
                .call("shortcutKeys", &(id,))
                .await
                .map_err(|error| error.to_string())?;
            if keys.iter().any(|sequence| sequence.0.first() == Some(&key)) {
                return Err(format!(
                    "That shortcut is already used by Reflow's {} action.",
                    id.get(3)
                        .or_else(|| id.get(1))
                        .map(String::as_str)
                        .unwrap_or("other")
                ));
            }
        }
        // The availability query excludes this component, so also check its
        // other actions using their real desktop bindings, not preferred keys.
        let available: bool = proxy
            .call("globalShortcutAvailable", &(key_sequence(key), app_id))
            .await
            .map_err(|error| error.to_string())?;
        if !available && !previous.contains(&key_sequence(key)) {
            return Err("That shortcut is already used by another desktop action. Choose another combination.".into());
        }
    }
    // This is the same configuration API used by KDE's shortcut editor. It
    // persists the binding and notifies the portal's owning QAction in place.
    proxy
        .call::<_, _, ()>("setForeignShortcutKeys", &(target, &requested))
        .await
        .map_err(|error| error.to_string())?;
    let assigned: Vec<KeySequence> = proxy
        .call("shortcutKeys", &(target,))
        .await
        .map_err(|error| error.to_string())?;
    if assigned != requested {
        proxy
            .call::<_, _, ()>("setForeignShortcutKeys", &(target, &previous))
            .await
            .map_err(|error| {
                format!(
                    "KDE rejected the new shortcut and could not restore the previous one: {error}"
                )
            })?;
        return Err(
            "KDE did not accept that combination. The previous shortcut was restored.".into(),
        );
    }
    Ok(())
}

pub async fn set_shortcut(
    app_id: &str,
    action: &str,
    shortcut: Option<&str>,
) -> Result<(), String> {
    let key = shortcut.map(qt_key).transpose()?;
    let connection = zbus::connection::Builder::session()
        .map_err(|error| error.to_string())?
        .method_timeout(Duration::from_secs(5))
        .build()
        .await
        .map_err(|error| error.to_string())?;
    let proxy = Proxy::new(
        &connection,
        "org.kde.kglobalaccel",
        "/kglobalaccel",
        "org.kde.KGlobalAccel",
    )
    .await
    .map_err(|error| error.to_string())?;
    assign(&proxy, app_id, action, key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_kde_keys_including_aliases_and_modifier_order() {
        assert_eq!(key_sequence(0x04000020), (vec![0x04000020, 0, 0, 0],));
        assert_eq!(qt_key("Ctrl+Space").unwrap(), 0x04000020);
        assert_eq!(qt_key("Shift+Control+Space").unwrap(), 0x06000020);
        assert_eq!(qt_key("Win+F8").unwrap(), 0x11000037);
        assert_eq!(qt_key("Ctrl+Alt+Backspace").unwrap(), 0x0d000003);
        assert_eq!(qt_key("Alt+ArrowLeft").unwrap(), 0x09000012);
        assert_eq!(qt_key("Ctrl+K").unwrap(), 0x0400004b);
        assert_eq!(qt_key("Ctrl+1").unwrap(), 0x04000031);
        assert!(qt_key("").is_err());
        assert!(qt_key("Ctrl+Shift").is_err());
        assert!(qt_key("Ctrl+NotAKey").is_err());
    }

    #[tokio::test]
    #[ignore = "requires REFLOW_ISOLATED_KDE_TEST=1 and a private D-Bus/KGlobalAccel service"]
    async fn kde_assignment_round_trip_and_conflicts() {
        use futures_util::FutureExt;
        assert_eq!(
            std::env::var("REFLOW_ISOLATED_KDE_TEST").as_deref(),
            Ok("1"),
            "Run against a private test service, never the desktop compositor"
        );
        let connection = zbus::connection::Builder::session()
            .unwrap()
            .method_timeout(Duration::from_secs(5))
            .build()
            .await
            .unwrap();
        let proxy = Proxy::new(
            &connection,
            "org.kde.kglobalaccel",
            "/kglobalaccel",
            "org.kde.KGlobalAccel",
        )
        .await
        .unwrap();
        let component = format!("com.reflow.app.test.{}", uuid::Uuid::new_v4().simple());
        for action in ["first", "second"] {
            proxy
                .call::<_, _, ()>(
                    "doRegister",
                    &(vec![
                        component.as_str(),
                        action,
                        "Reflow shortcut test",
                        action,
                    ],),
                )
                .await
                .unwrap();
            // Registration is an intermediate state until the initial key
            // set is supplied; KGlobalAccel omits fresh actions from queries.
            let _: Vec<KeySequence> = proxy
                .call(
                    "setShortcutKeys",
                    &(
                        vec![component.as_str(), action, "Reflow shortcut test", action],
                        Vec::<KeySequence>::new(),
                        4u32,
                    ),
                )
                .await
                .unwrap();
        }
        // Always remove only this UUID-scoped component's actions, even when
        // an assertion fails. No existing user binding is changed by this test.
        let outcome = std::panic::AssertUnwindSafe(async {
            let mut free = None;
            for function in 12..=24 {
                let key = qt_key(&format!("Ctrl+Alt+Shift+F{function}")).unwrap();
                let available: bool = proxy
                    .call(
                        "globalShortcutAvailable",
                        &(key_sequence(key), component.as_str()),
                    )
                    .await
                    .unwrap();
                if available {
                    free = Some(key);
                    break;
                }
            }
            let key = free.expect("one unused test key must be available");
            assign(&proxy, &component, "first", Some(key))
                .await
                .unwrap();
            let actual: Vec<KeySequence> = proxy
                .call(
                    "shortcutKeys",
                    &(vec![
                        component.as_str(),
                        "first",
                        "Reflow shortcut test",
                        "first",
                    ],),
                )
                .await
                .unwrap();
            assert_eq!(actual, vec![key_sequence(key)]);
            let error = assign(&proxy, &component, "second", Some(key))
                .await
                .unwrap_err();
            assert!(error.contains("Reflow's"), "{error}");
            let unchanged: Vec<KeySequence> = proxy
                .call(
                    "shortcutKeys",
                    &(vec![
                        component.as_str(),
                        "first",
                        "Reflow shortcut test",
                        "first",
                    ],),
                )
                .await
                .unwrap();
            assert_eq!(unchanged, actual);
            assert!(assign(&proxy, &component, "unknown", Some(key))
                .await
                .is_err());
            assign(&proxy, &component, "first", None).await.unwrap();
            assign(&proxy, &component, "second", Some(key))
                .await
                .unwrap();
            assign(&proxy, &component, "second", None).await.unwrap();
        })
        .catch_unwind()
        .await;
        for action in ["first", "second"] {
            let removed: bool = proxy
                .call("unregister", &(component.as_str(), action))
                .await
                .unwrap();
            assert!(removed);
        }
        outcome.unwrap();
    }
}
