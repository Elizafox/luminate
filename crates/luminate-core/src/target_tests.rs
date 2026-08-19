// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn target_constructors_build_expected_ids() {
    assert_eq!(
        TargetId::device("dev"),
        TargetId::Device(DeviceId::new("dev"))
    );
    assert_eq!(
        TargetId::surface("dev", "keys"),
        TargetId::Surface {
            device: DeviceId::new("dev"),
            surface: SurfaceId::new("keys"),
        }
    );
    assert_eq!(
        TargetId::element("dev", "keys", "escape"),
        TargetId::Element {
            device: DeviceId::new("dev"),
            surface: SurfaceId::new("keys"),
            element: ElementId::new("escape"),
        }
    );
    assert_eq!(
        TargetId::group("dev", "all"),
        TargetId::Group {
            device: DeviceId::new("dev"),
            group: GroupId::new("all"),
        }
    );
}

#[test]
fn target_parts_cover_valid_shapes_and_reject_ambiguous_combinations() {
    assert_eq!(
        TargetId::from_parts("dev", None::<String>, None::<String>, None::<String>),
        Ok(TargetId::device("dev"))
    );
    assert_eq!(
        TargetId::from_parts("dev", Some("keys"), None::<String>, None::<String>),
        Ok(TargetId::surface("dev", "keys"))
    );
    assert_eq!(
        TargetId::from_parts("dev", Some("keys"), Some("escape"), None::<String>),
        Ok(TargetId::element("dev", "keys", "escape"))
    );
    assert_eq!(
        TargetId::from_parts("dev", None::<String>, None::<String>, Some("all")),
        Ok(TargetId::group("dev", "all"))
    );

    assert_eq!(
        TargetId::from_parts("dev", None::<String>, Some("escape"), None::<String>),
        Err("element target requires a surface identifier")
    );
    assert_eq!(
        TargetId::from_parts("dev", Some("keys"), None::<String>, Some("all")),
        Err("group target cannot be combined with a surface or element target")
    );

    for target in [
        TargetId::device("dev"),
        TargetId::surface("dev", "keys"),
        TargetId::element("dev", "keys", "escape"),
        TargetId::group("dev", "all"),
    ] {
        assert_eq!(target.device_id().as_str(), "dev");
    }
}
