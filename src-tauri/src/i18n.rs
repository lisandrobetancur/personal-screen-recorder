//! Idioma de la app: español si el sistema está en español; inglés en
//! cualquier otro caso. `PSR_LANG=es|en` lo fuerza (útil para probar).

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

pub fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| {
        let locale = std::env::var("PSR_LANG")
            .ok()
            .or_else(sys_locale::get_locale)
            .unwrap_or_default()
            .to_lowercase();
        if locale.starts_with("es") { Lang::Es } else { Lang::En }
    })
}

/// Código del idioma para la interfaz ("es" o "en").
pub fn code() -> &'static str {
    match lang() {
        Lang::Es => "es",
        Lang::En => "en",
    }
}

/// Elige el texto según el idioma: `tr!("español", "English")`.
#[macro_export]
macro_rules! tr {
    ($es:expr, $en:expr $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::Es => $es,
            $crate::i18n::Lang::En => $en,
        }
    };
}
