// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::path::PathBuf;

use super::*;

#[test]
fn render_embeds_the_program_and_log_paths() {
    let rendered = render(
        &PathBuf::from("/test/bin/luminated"),
        &PathBuf::from("/test/log/luminated.log"),
    );
    assert!(rendered.contains("<string>/test/bin/luminated</string>"));
    assert!(rendered.contains("<string>/test/log/luminated.log</string>"));
    assert!(rendered.contains("<string>com.wilcoxti.luminate.luminated</string>"));
    assert!(rendered.contains("<string>_luminated</string>"));
    assert!(rendered.contains("<string>_luminate</string>"));
}
