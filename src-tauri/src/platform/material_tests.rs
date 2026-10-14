use super::macos_vibrancy_material;
use window_vibrancy::NSVisualEffectMaterial;

#[test]
fn uses_the_fixed_original_material_for_recognized_darwin_releases() {
    for version in ["18.0.0", "18.7.0", "19.6.0", "20.0.0", "23.0.0", "25.0.0"] {
        assert_eq!(
            macos_vibrancy_material(Some(version)),
            NSVisualEffectMaterial::FullScreenUI,
            "{version}"
        );
    }
    for version in ["17.0.0", "17.7.0", "16.0.0", "26.0.0", "27.0.0", "999.0.0"] {
        assert_eq!(
            macos_vibrancy_material(Some(version)),
            NSVisualEffectMaterial::Dark,
            "{version}"
        );
    }
}

#[test]
fn retains_the_original_dark_fallback_when_the_os_version_is_unavailable() {
    for version in [None, Some(""), Some("invalid")] {
        assert_eq!(
            macos_vibrancy_material(version),
            NSVisualEffectMaterial::Dark,
            "{version:?}"
        );
    }
}
