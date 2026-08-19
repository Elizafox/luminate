// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Supported-product registry lookup and capability tests.

use super::*;

#[test]
fn registry_includes_owned_plain_bulb_families() {
    assert_eq!(
        product(1, 23).map(|product| product.name),
        Some("LIFX (A19)")
    );
    assert_eq!(
        product(1, 24).map(|product| product.name),
        Some("LIFX (BR30)")
    );
    assert_eq!(product(1, 72).map(|product| product.name), Some("LIFX A19"));
}

#[test]
fn registry_includes_z_and_beam_as_linear_products() {
    assert_eq!(
        product(1, 31).map(|product| product.topology),
        Some(ProductTopology::Linear)
    );
    assert_eq!(
        product(1, 38).map(|product| product.topology),
        Some(ProductTopology::Linear)
    );
    assert_eq!(
        product(1, 117).map(|product| product.topology),
        Some(ProductTopology::Linear)
    );
    assert_eq!(
        product(1, 120).map(|product| product.topology),
        Some(ProductTopology::Linear)
    );
}

#[test]
fn registry_distinguishes_linear_product_physical_shapes() {
    for product_id in [31, 32, 117, 118] {
        assert_eq!(
            product(1, product_id).map(|product| product.physical_tags),
            Some(["shape:flexible-strip"].as_slice()),
            "product {product_id}"
        );
    }
    for product_id in [38, 56, 119, 120] {
        assert_eq!(
            product(1, product_id).map(|product| product.physical_tags),
            Some(["shape:modular-light-bar"].as_slice()),
            "product {product_id}"
        );
    }
}

#[test]
fn registry_curates_plain_product_forms_by_product_id() {
    for (product_ids, physical_tag) in [
        (A19_PRODUCTS, "shape:a19"),
        (BR30_PRODUCTS, "shape:br30"),
        (GU10_PRODUCTS, "shape:gu10"),
        (PAR38_PRODUCTS, "shape:par38"),
        (DOWNLIGHT_PRODUCTS, "shape:downlight"),
        (CANDLE_PRODUCTS, "shape:candle"),
    ] {
        for product_id in product_ids {
            assert_eq!(
                product(1, *product_id).map(|product| product.physical_tags),
                Some([physical_tag].as_slice()),
                "product {product_id}"
            );
        }
    }

    assert_eq!(
        product(1, 1).map(|product| product.physical_tags),
        Some([].as_slice())
    );
}

#[test]
fn registry_excludes_matrix_switch_and_unknown_products() {
    assert_eq!(product(1, 55), None); // Tile
    assert_eq!(product(1, 70), None); // Switch
    assert_eq!(product(1, u32::MAX), None);
}
