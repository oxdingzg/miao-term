//! Tiny i18n helper shared by the hosts (English default, Simplified Chinese).

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Zh,
}

impl Lang {
    pub fn parse(s: Option<&str>) -> Lang {
        match s {
            Some(v) if v.to_ascii_lowercase().starts_with("zh") => Lang::Zh,
            _ => Lang::En,
        }
    }
}

/// Pick the English or Chinese string for `lang`.
pub fn t(lang: Lang, en: &'static str, zh: &'static str) -> &'static str {
    if lang == Lang::Zh {
        zh
    } else {
        en
    }
}
