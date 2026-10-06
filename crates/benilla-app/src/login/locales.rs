//! First localization milestone: candidate discovery and persistent preference.
//! Switching language reloads the page so all resources/VM/session agree.

use super::*;
use crate::web_config::text;
use benilla_assets::RawFile;
use benilla_formats::locales::{LocaleCatalog, LOCALE_CODES};

#[derive(Default)]
pub(super) struct LocaleChoice {
    pub codes: Vec<String>,
    pub selected: String,
    pub status: String,
    pub open: bool,
    pub revision: u64,
}

pub(super) fn install(app: &mut App) {
    let selected = crate::web_config::active_locale();
    assert!(
        LOCALE_CODES.contains(&selected.as_str()),
        "unsupported startup locale"
    );
    app.world_mut().resource_mut::<LoginForm>().locale = LocaleChoice {
        selected,
        status: text("Detecting locale data...", "Поиск языковых данных…").into(),
        ..default()
    };
    app.add_systems(Update, deliver);
}

fn deliver(
    server: Res<AssetServer>,
    files: Res<Assets<RawFile>>,
    mut form: ResMut<LoginForm>,
    mut request: Local<Option<Handle<RawFile>>>,
    mut finished: Local<bool>,
) {
    if *finished {
        return;
    }
    let handle = request.get_or_insert_with(|| server.load("locale-catalog://locales.bli"));
    let result = if let Some(file) = files.get(&*handle) {
        Some(LocaleCatalog::decode(&file.0).map_err(|error| error.to_string()))
    } else if let Some(bevy::asset::LoadState::Failed(error)) = server.get_load_state(handle.id()) {
        Some(Err(error.to_string()))
    } else {
        None
    };
    let Some(result) = result else { return };
    match result {
        Ok(catalog) => {
            info!("login: detected locale data candidates {:?}", catalog.codes);
            form.locale.codes = catalog
                .codes
                .into_iter()
                .filter(|code| matches!(code.as_str(), "enUS" | "ruRU"))
                .collect();
            form.locale.status = text(
                "Language changes reload the page.",
                "Смена языка перезагрузит страницу.",
            )
            .into();
        }
        Err(error) => {
            warn!("login: locale catalog unavailable: {error}");
            form.locale.status = text(
                "Locale list unavailable; refresh development gateway.",
                "Список языков недоступен; перезапустите шлюз разработки.",
            )
            .into();
        }
    }
    form.locale.revision += 1;
    *finished = true;
}

pub(super) fn select(form: &mut LoginForm, index: usize) {
    let Some(code) = form.locale.codes.get(index).cloned() else {
        return;
    };
    let result = crate::local_state::locale_choice_path()
        .ok_or_else(|| std::io::Error::other("local settings unavailable"))
        .and_then(|path| crate::local_state::write_atomic(&path, &code));
    match result {
        Ok(()) => {
            let changed = code != crate::web_config::active_locale();
            form.locale.selected = code;
            form.locale.status = text("Language saved.", "Язык сохранён.").into();
            if changed {
                form.locale.status = text(
                    "Reloading with selected language...",
                    "Перезагрузка с выбранным языком…",
                )
                .into();
                if let Some(window) = web_sys::window() {
                    if let Err(error) = window.location().reload() {
                        warn!("login: language reload failed: {error:?}");
                        form.locale.status = text(
                            "Saved; reload the page to apply language.",
                            "Сохранено; перезагрузите страницу для смены языка.",
                        )
                        .into();
                    }
                }
            }
        }
        Err(error) => {
            warn!("login: saving locale preference failed: {error}");
            form.locale.status = text(
                "Could not save language preference.",
                "Не удалось сохранить язык.",
            )
            .into();
        }
    }
    form.locale.open = false;
    form.locale.revision += 1;
}
