//! KDE keyboard layouts, to type scripts the active layout has no keys for.

use zbus::blocking::{Connection, Proxy};

pub struct Layouts {
    proxy: Proxy<'static>,
    /// Short names such as `us` and `il`, in KDE's order.
    names: Vec<String>,
    pub active: u32,
}

impl Layouts {
    pub fn new() -> Option<Layouts> {
        let proxy = Proxy::new(
            &Connection::session().ok()?,
            "org.kde.keyboard",
            "/Layouts",
            "org.kde.KeyboardLayouts",
        )
        .ok()?;
        let listed: Vec<(String, String, String)> = proxy.call("getLayoutsList", &()).ok()?;
        let active = proxy.call("getLayout", &()).ok()?;
        Some(Layouts {
            proxy,
            names: listed.into_iter().map(|(name, _, _)| name).collect(),
            active,
        })
    }

    pub fn set(&self, index: u32) -> bool {
        self.proxy.call("setLayout", &(index,)).unwrap_or(false)
    }

    /// The layout that types `character`: `latin` for ASCII letters, one of the
    /// installed layouts for other scripts, `None` for what any layout types.
    pub fn for_char(&self, character: char, latin: u32) -> Option<u32> {
        if character.is_ascii_alphabetic() {
            return Some(latin);
        }
        let wanted: &[&str] = match character as u32 {
            0x0590..=0x05FF => &["il"],
            0x0600..=0x06FF => &["ara", "ir", "af", "pk"],
            0x0400..=0x04FF => &["ru", "ua", "by", "bg", "rs", "mk", "kz"],
            0x0370..=0x03FF => &["gr"],
            _ => return None,
        };
        let index = self
            .names
            .iter()
            .position(|name| wanted.contains(&name.as_str()))?;
        Some(index as u32)
    }
}
