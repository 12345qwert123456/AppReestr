//! AppReestr core: discovers installed applications from every source, decides what counts as
//! an application, and plans safe removals. No GUI code lives here.

pub mod appstream;
pub mod cache;
pub mod classify;
pub mod config;
pub mod desktop;
pub mod i18n;
pub mod leftovers;
pub mod listing;
pub mod merge;
pub mod model;
pub mod plan;
pub mod protect;
pub mod reasons;
pub mod rules;
pub mod scan;
pub mod sources;
pub mod sysroot;

pub const APP_ID: &str = "io.github.appreestr.AppReestr";
