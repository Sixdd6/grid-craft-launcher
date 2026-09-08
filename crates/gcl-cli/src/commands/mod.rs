//! One module per top-level subcommand. Each maps its arguments onto `Launcher` methods.

pub mod account;
pub mod config;
pub mod content;
pub mod debug;
pub mod instance;
pub mod java;
pub mod launch;
pub mod loader;
pub mod modpack;
pub mod settings;
pub mod version;

use clap::ValueEnum;
use gcl_core::instances::model::GcPreset;

/// Garbage collector preset as spelled on the command line.
///
/// One value per [`GcPreset`], spelled the way `instance.toml` and `config.toml` store it, so
/// `gcl config set-jvm --gc`, `gcl instance jvm --gc`, and the saved file all read the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum GcPresetArg {
    /// No collector flag: the JVM picks its own.
    Default,
    /// Serial collector.
    Serial,
    /// Parallel collector.
    Parallel,
    /// G1 collector.
    G1,
    /// ZGC, non-generational where the JVM still has that mode.
    Zgc,
    /// Generational ZGC.
    #[value(name = "zgc_generational", alias = "zgc-generational")]
    ZgcGenerational,
    /// Shenandoah collector.
    Shenandoah,
}

impl From<GcPresetArg> for GcPreset {
    fn from(arg: GcPresetArg) -> GcPreset {
        match arg {
            GcPresetArg::Default => GcPreset::Default,
            GcPresetArg::Serial => GcPreset::Serial,
            GcPresetArg::Parallel => GcPreset::Parallel,
            GcPresetArg::G1 => GcPreset::G1,
            GcPresetArg::Zgc => GcPreset::Zgc,
            GcPresetArg::ZgcGenerational => GcPreset::ZgcGenerational,
            GcPresetArg::Shenandoah => GcPreset::Shenandoah,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gc_preset_arg_maps_onto_the_core_preset_of_the_same_name() {
        let mapped: Vec<GcPreset> = GcPresetArg::value_variants()
            .iter()
            .map(|arg| GcPreset::from(*arg))
            .collect();
        assert_eq!(mapped, GcPreset::all(), "one arg per preset, in menu order");
    }

    #[test]
    fn a_gc_preset_arg_is_spelled_the_way_the_config_file_stores_it() {
        for arg in GcPresetArg::value_variants() {
            let token = arg
                .to_possible_value()
                .expect("every variant is selectable")
                .get_name()
                .to_string();
            assert_eq!(token, GcPreset::from(*arg).to_string());
        }
    }
}
