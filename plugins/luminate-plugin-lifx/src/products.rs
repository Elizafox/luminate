// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Embedded supported subset of LIFX's official product registry.
//!
//! Generated from `LIFX/products` on 2026-07-12 (source JSON SHA-256
//! `82a430a2bee30bb6e514987c8c5d3f203a64714b78bd7fb0f02778d445b95395`).
//! The registry exposes plain colour and linear multizone products, but not
//! matrix, chain, relay, or button topology. Unknown products remain
//! undiscovered rather than being assigned capabilities optimistically.

const PLAIN_COLOUR_PRODUCTS: &[(u32, &str)] = &[
    (1, "Original"),
    (2, "Original 1000"),
    (3, "Color 650"),
    (14, "LIFX Carbon Prototype"),
    (15, "LIFX LCMv4 Color (A21)"),
    (20, "LIFX Color 1000 (BR30)"),
    (21, "LIFX LCMv4 Color (GU10)"),
    (22, "LIFX Color 1000"),
    (23, "LIFX (A19)"),
    (24, "LIFX (BR30)"),
    (25, "LIFX+ (A19)"),
    (26, "LIFX+ (BR30)"),
    (27, "LIFX (A19)"),
    (28, "LIFX (BR30)"),
    (29, "LIFX+ (A19)"),
    (30, "LIFX+ (BR30)"),
    (33, "LIFX Downlight"),
    (36, "LIFX DL"),
    (37, "LIFX DL"),
    (40, "LIFX DL Colour 700lm"),
    (43, "LIFX (A19)"),
    (44, "LIFX (BR30)"),
    (45, "LIFX+ (A19)"),
    (46, "LIFX+ (BR30)"),
    (49, "LIFX Mini C"),
    (52, "LIFX GU10"),
    (53, "LIFX GU10"),
    (59, "LIFX Mini C"),
    (62, "LIFX (A19) LCM3"),
    (63, "LIFX (BR30) LCM3"),
    (64, "LIFX+ (A19) LCM3"),
    (65, "LIFX+ (BR30) LCM3"),
    (72, "LIFX A19"),
    (90, "LIFX Clean A19 1100lm"),
    (91, "LIFX Color 800lm"),
    (92, "LIFX Colour 1000lm"),
    (93, "LIFX Color A19 1100lm"),
    (94, "LIFX Color BR30 1100lm"),
    (95, "Copper"),
    (97, "LIFX Colour A19 1200lm"),
    (98, "LIFX Colour BR30 1100lm"),
    (99, "LIFX Clean A19 1200lm"),
    (109, "LIFX NV A19 1100lm"),
    (110, "LIFX NV BR30 1100lm"),
    (111, "LIFX NV A19 1200lm"),
    (112, "LIFX NV BR30 1100lm"),
    (121, "LIFX DL"),
    (122, "LIFX DL"),
    (123, "LIFX Mini 3.1 Color US"),
    (124, "LIFX Mini 3.1 Color Intl"),
    (129, "LIFX Color 800"),
    (130, "LIFX Colour 1000"),
    (135, "LIFX GU10 Color US"),
    (136, "LIFX GU10 Color Intl"),
    (153, "LIFX PAR38 US"),
    (154, "LIFX PAR38 Intl"),
    (155, "LIFX Sunshine"),
    (156, "LIFX A21 1000lm Intl"),
    (163, "LIFX A19"),
    (164, "LIFX BR30"),
    (165, "LIFX A19 Intl"),
    (166, "LIFX BR30 Intl"),
    (167, "LIFX DL AU"),
    (168, "LIFX DL US"),
    (169, "LIFX A21"),
    (170, "LIFX A21"),
    (175, "LIFX PAR38"),
    (178, "LIFX Downlight US"),
    (179, "LIFX Downlight US"),
    (180, "LIFX Downlight US"),
    (181, "LIFX Mini"),
    (182, "LIFX Mini"),
    (187, "LIFX Candle"),
    (188, "LIFX Candle Intl"),
    (191, "LIFX Everyday A19"),
    (192, "LIFX Everyday A19 Intl"),
    (223, "LIFX DL"),
    (224, "LIFX DL Intl"),
    (225, "LIFX PAR38 Intl"),
];

const LINEAR_PRODUCTS: &[(u32, &str, &[&str])] = &[
    (31, "LIFX Z", &["shape:flexible-strip"]),
    (32, "LIFX Z", &["shape:flexible-strip"]),
    (38, "LIFX Beam", &["shape:modular-light-bar"]),
    (56, "LIFX Beam", &["shape:modular-light-bar"]),
    (117, "LIFX Z", &["shape:flexible-strip"]),
    (118, "LIFX Z", &["shape:flexible-strip"]),
    (119, "LIFX Beam", &["shape:modular-light-bar"]),
    (120, "LIFX Beam", &["shape:modular-light-bar"]),
];

const A19_PRODUCTS: &[u32] = &[
    23, 25, 27, 29, 43, 45, 62, 64, 72, 90, 93, 97, 99, 109, 111, 163, 165, 191, 192,
];
const BR30_PRODUCTS: &[u32] = &[
    20, 24, 26, 28, 30, 44, 46, 63, 65, 94, 98, 110, 112, 164, 166,
];
const GU10_PRODUCTS: &[u32] = &[21, 52, 53, 135, 136];
const PAR38_PRODUCTS: &[u32] = &[153, 154, 175, 225];
const DOWNLIGHT_PRODUCTS: &[u32] = &[33, 36, 37, 40, 121, 122, 167, 168, 178, 179, 180, 223, 224];
const CANDLE_PRODUCTS: &[u32] = &[187, 188];

fn plain_physical_tags(product: u32) -> &'static [&'static str] {
    if A19_PRODUCTS.contains(&product) {
        &["shape:a19"]
    } else if BR30_PRODUCTS.contains(&product) {
        &["shape:br30"]
    } else if GU10_PRODUCTS.contains(&product) {
        &["shape:gu10"]
    } else if PAR38_PRODUCTS.contains(&product) {
        &["shape:par38"]
    } else if DOWNLIGHT_PRODUCTS.contains(&product) {
        &["shape:downlight"]
    } else if CANDLE_PRODUCTS.contains(&product) {
        &["shape:candle"]
    } else {
        &[]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductTopology {
    Plain,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Product {
    pub name: &'static str,
    pub topology: ProductTopology,
    pub physical_tags: &'static [&'static str],
}

pub fn product(vendor: u32, product: u32) -> Option<Product> {
    if vendor != 1 {
        return None;
    }
    if let Some(name) = PLAIN_COLOUR_PRODUCTS
        .iter()
        .find_map(|(id, name)| (*id == product).then_some(*name))
    {
        return Some(Product {
            name,
            topology: ProductTopology::Plain,
            physical_tags: plain_physical_tags(product),
        });
    }
    LINEAR_PRODUCTS
        .iter()
        .find_map(|(id, name, physical_tags)| (*id == product).then_some((*name, *physical_tags)))
        .map(|(name, physical_tags)| Product {
            name,
            topology: ProductTopology::Linear,
            physical_tags,
        })
}

#[cfg(test)]
#[path = "products_tests.rs"]
mod tests;
