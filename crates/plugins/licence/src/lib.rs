//! Licence file type plugin: core and presentation halves.
//!
//! What a repository is licensed under is a fact about the repository, not
//! a rendering of a file's bytes: the file is plain text, so without this
//! plugin the one question every reader of an unfamiliar checkout asks -
//! what am I allowed to do with this - is answered by reading several
//! hundred lines of boilerplate by eye.
//!
//! The real file is named `LICENSE`, `LICENCE`, `COPYING` or `NOTICE`, with
//! or without an extension - but `PluginCore::sniff` only ever receives a
//! content prefix, never a path (`crates/plugin-api/src/lib.rs`), the same
//! conflict the Dockerfile (#39), Makefile (#38) and `.gitmodules` (#708)
//! plugins hit. Resolved the same way here: the distinctive body phrases of
//! each licence decide, not the file name, so a `NOTICE` holding something
//! else is never claimed.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot. None: the
/// real file is recognised by its body, which sniffing can see, not by an
/// extension it usually does not have.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One licence this plugin recognises, and the phrases that decide it.
///
/// `sniff` alone says a file is this licence: every phrase in it, present
/// anywhere in the body (case-sensitive, after collapsing whitespace so
/// line-wrapping does not matter), is distinctive enough that no other
/// licence or ordinary prose carries it by accident. `canonical` is the
/// larger set the unmodified text carries beyond that - missing even one
/// means the body has been edited, which is what
/// [`LicenceView::differs_from_canonical`] reports.
struct Kind {
    /// The identifier reported in [`LicenceView::licences`].
    id: &'static str,
    /// Phrases that, all present, mean the file is this licence.
    sniff: &'static [&'static str],
    /// Phrases the unmodified licence text carries beyond `sniff`'s own.
    canonical: &'static [&'static str],
}

/// Every licence this plugin recognises, in no particular order - matches
/// are sorted by where they occur in the file, not by this list.
const KINDS: &[Kind] = &[
    Kind {
        id: "MIT",
        sniff: &[
            "Permission is hereby granted, free of charge, to any person \
             obtaining a copy of this software and associated documentation \
             files (the \"Software\"), to deal in the Software without \
             restriction, including without limitation the rights to use, \
             copy, modify, merge, publish, distribute, sublicense, and/or \
             sell copies of the Software, and to permit persons to whom the \
             Software is furnished to do so, subject to the following \
             conditions:",
        ],
        canonical: &[
            "The above copyright notice and this permission notice shall be \
             included in all copies or substantial portions of the Software.",
            "THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY \
             KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE \
             WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR \
             PURPOSE AND NONINFRINGEMENT.",
            "IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE \
             FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION \
             OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN \
             CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN \
             THE SOFTWARE.",
        ],
    },
    Kind {
        id: "Apache-2.0",
        sniff: &[
            "Apache License",
            "Version 2.0, January 2004",
            "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION",
        ],
        canonical: &[
            "END OF TERMS AND CONDITIONS",
            "APPENDIX: How to apply the Apache License to your work.",
            "Licensed under the Apache License, Version 2.0 (the \"License\"); \
             you may not use this file except in compliance with the \
             License. You may obtain a copy of the License at",
            "Unless required by applicable law or agreed to in writing, \
             software distributed under the License is distributed on an \
             \"AS IS\" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, \
             either express or implied.",
        ],
    },
    Kind {
        id: "BSD-2-Clause",
        sniff: &[
            "Redistribution and use in source and binary forms, with or \
             without modification, are permitted provided that the \
             following conditions are met:",
            "Redistributions of source code must retain the above copyright \
             notice, this list of conditions and the following disclaimer.",
            "Redistributions in binary form must reproduce the above \
             copyright notice, this list of conditions and the following \
             disclaimer in the documentation and/or other materials \
             provided with the distribution.",
        ],
        canonical: &[
            "THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDER \"AS IS\" AND \
             ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED \
             TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A \
             PARTICULAR PURPOSE ARE DISCLAIMED.",
        ],
    },
    Kind {
        id: "BSD-3-Clause",
        sniff: &[
            "Redistribution and use in source and binary forms, with or \
             without modification, are permitted provided that the \
             following conditions are met:",
            "Redistributions of source code must retain the above copyright \
             notice, this list of conditions and the following disclaimer.",
            "Redistributions in binary form must reproduce the above \
             copyright notice, this list of conditions and the following \
             disclaimer in the documentation and/or other materials \
             provided with the distribution.",
            "Neither the name of",
        ],
        canonical: &[
            "Neither the name of the copyright holder nor the names of its \
             contributors may be used to endorse or promote products \
             derived from this software without specific prior written \
             permission.",
            "THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDER AND \
             CONTRIBUTORS \"AS IS\" AND ANY EXPRESS OR IMPLIED WARRANTIES, \
             INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF \
             MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE \
             DISCLAIMED.",
        ],
    },
    Kind {
        id: "GPL-2.0",
        sniff: &["GNU GENERAL PUBLIC LICENSE", "Version 2, June 1991"],
        canonical: &[
            "This program is free software; you can redistribute it and/or \
             modify it under the terms of the GNU General Public License as \
             published by the Free Software Foundation; either version 2 of \
             the License, or (at your option) any later version.",
            "This program is distributed in the hope that it will be \
             useful, but WITHOUT ANY WARRANTY; without even the implied \
             warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR \
             PURPOSE.",
        ],
    },
    Kind {
        id: "GPL-3.0",
        sniff: &["GNU GENERAL PUBLIC LICENSE", "Version 3, 29 June 2007"],
        canonical: &[
            "This program is free software: you can redistribute it and/or \
             modify it under the terms of the GNU General Public License as \
             published by the Free Software Foundation, either version 3 of \
             the License, or (at your option) any later version.",
            "This program is distributed in the hope that it will be \
             useful, but WITHOUT ANY WARRANTY; without even the implied \
             warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR \
             PURPOSE.",
        ],
    },
    Kind {
        id: "LGPL-2.1",
        sniff: &[
            "GNU LESSER GENERAL PUBLIC LICENSE",
            "Version 2.1, February 1999",
        ],
        canonical: &[
            "This library is free software; you can redistribute it and/or \
             modify it under the terms of the GNU Lesser General Public \
             License as published by the Free Software Foundation; either \
             version 2.1 of the License, or (at your option) any later \
             version.",
        ],
    },
    Kind {
        id: "LGPL-3.0",
        sniff: &[
            "GNU LESSER GENERAL PUBLIC LICENSE",
            "Version 3, 29 June 2007",
        ],
        canonical: &[
            "This library is free software: you can redistribute it and/or \
             modify it under the terms of the GNU Lesser General Public \
             License as published by the Free Software Foundation, either \
             version 3 of the License, or (at your option) any later \
             version.",
        ],
    },
    Kind {
        id: "AGPL-3.0",
        sniff: &[
            "GNU AFFERO GENERAL PUBLIC LICENSE",
            "Version 3, 19 November 2007",
        ],
        canonical: &[
            "This program is free software: you can redistribute it and/or \
             modify it under the terms of the GNU Affero General Public \
             License as published by the Free Software Foundation, either \
             version 3 of the License, or (at your option) any later \
             version.",
        ],
    },
    Kind {
        id: "MPL-2.0",
        sniff: &[
            "Mozilla Public License Version 2.0",
            "This Source Code Form is subject to the terms of the Mozilla \
             Public License, v. 2.0. If a copy of the MPL was not \
             distributed with this file, You can obtain one at \
             http://mozilla.org/MPL/2.0/.",
        ],
        canonical: &["Exhibit A - Source Code Form License Notice"],
    },
    Kind {
        id: "Unlicense",
        sniff: &["This is free and unencumbered software released into the public domain."],
        canonical: &[
            "Anyone is free to copy, modify, publish, use, compile, sell, or \
             distribute this software, either in source code form or as a \
             compiled binary, for any purpose, commercial or \
             non-commercial, and by any means.",
            "In jurisdictions that recognize copyright laws, the author or \
             authors of this software dedicate any and all copyright \
             interest in the software to the public domain.",
        ],
    },
];

/// View data produced by [`LicenceCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenceView {
    /// The licences recognised, in the order their first marker appears.
    /// More than one entry means the file holds more than one licence.
    pub licences: Vec<String>,
    /// The copyright holder on the first genuine copyright notice, if one
    /// is present.
    pub holder: Option<String>,
    /// The year, or year range, on that same notice.
    pub years: Option<String>,
    /// Whether the body is missing a phrase the unmodified text of every
    /// licence it matched carries - true means the wording was edited.
    pub differs_from_canonical: bool,
    /// The file's text, so the plain view can show it.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// `text` with every run of whitespace - including a line break - collapsed
/// to one space, so a licence reflowed to a different column width still
/// matches phrases written here as one line.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(ch);
            in_space = false;
        }
    }
    out.trim().to_owned()
}

/// Every licence [`KINDS`] recognises in `normalized`, as `(id, differs)`
/// pairs ordered by where each one's first marker occurs.
///
/// A 3-clause BSD licence carries every marker the 2-clause form does, so
/// after matching, a 3-clause match drops the 2-clause one rather than
/// reporting a file as holding both.
fn detect(normalized: &str) -> Vec<(&'static str, bool)> {
    let mut found: Vec<(usize, &'static str, bool)> = KINDS
        .iter()
        .filter_map(|kind| {
            if !kind.sniff.iter().all(|marker| normalized.contains(marker)) {
                return None;
            }
            let position = normalized.find(kind.sniff[0]).unwrap_or(usize::MAX);
            let differs = !kind
                .canonical
                .iter()
                .all(|marker| normalized.contains(marker));
            Some((position, kind.id, differs))
        })
        .collect();

    if found.iter().any(|(_, id, _)| *id == "BSD-3-Clause") {
        found.retain(|(_, id, _)| *id != "BSD-2-Clause");
    }

    found.sort_by_key(|(position, _, _)| *position);
    found
        .into_iter()
        .map(|(_, id, differs)| (id, differs))
        .collect()
}

/// Whether `word` (with a trailing comma stripped) is a year or a hyphenated
/// year range, e.g. `2021` or `2019-2024`.
fn is_year_token(word: &str) -> bool {
    let word = word.trim_end_matches(',');
    let is_year = |s: &str| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit());
    match word.split_once('-') {
        Some((from, to)) => is_year(from) && is_year(to),
        None => is_year(word),
    }
}

/// The years and the holder in a copyright notice's text, once the leading
/// `copyright` word and any `(c)`/`©` mark have already been stripped.
fn split_years_and_holder(rest: &str) -> (Option<String>, Option<String>) {
    let mut rest = rest.trim_start();
    for mark in ["(c)", "(C)", "©"] {
        if let Some(stripped) = rest.strip_prefix(mark) {
            rest = stripped.trim_start();
            break;
        }
    }

    let words: Vec<&str> = rest.split_whitespace().collect();
    let year_count = words.iter().take_while(|word| is_year_token(word)).count();
    let years = (year_count > 0).then(|| words[..year_count].join(" "));

    let holder = words[year_count..].join(" ");
    let holder = holder.trim().trim_end_matches(',').trim();
    let holder = (!holder.is_empty()).then(|| holder.to_owned());

    (years, holder)
}

/// The text after `copyright` on the first line that is a genuine notice
/// about the file being viewed, or `None` if there is no such line.
///
/// The GNU licences (GPL, LGPL, AGPL) open with the Free Software
/// Foundation's own copyright on the licence *document*, immediately
/// followed by "Everyone is permitted to copy and distribute verbatim
/// copies of this license document, but changing it is not allowed." - a
/// notice about the text, not about whatever it is licensing. Skipped so
/// it never gets reported as the file's own holder.
fn copyright_notice(text: &str) -> Option<&str> {
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("copyright") else {
            continue;
        };
        let word_boundary = rest
            .chars()
            .next()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric());
        if !word_boundary {
            continue;
        }

        let next_line = lines[index + 1..]
            .iter()
            .map(|line| line.trim())
            .find(|line| !line.is_empty());
        if next_line.is_some_and(|line| {
            line.starts_with("Everyone is permitted to copy and distribute verbatim copies")
        }) {
            continue;
        }

        return Some(&trimmed[trimmed.len() - rest.len()..]);
    }
    None
}

/// Everything [`LicenceView`] holds, read from `text`.
fn parse(text: &str) -> LicenceView {
    let normalized = normalize(text);
    let matches = detect(&normalized);
    let licences = matches.iter().map(|(id, _)| (*id).to_owned()).collect();
    let differs_from_canonical = matches.iter().any(|(_, differs)| *differs);
    let (years, holder) = copyright_notice(text).map_or((None, None), split_years_and_holder);

    LicenceView {
        licences,
        holder,
        years,
        differs_from_canonical,
        content: String::new(),
        truncated: false,
    }
}

/// The licence plugin's core half.
#[derive(Debug, Default)]
pub struct LicenceCore;

impl PluginCore for LicenceCore {
    fn name(&self) -> &'static str {
        "licence"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(|text| !detect(&normalize(text)).is_empty())
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let mut view = parse(&content);
        view.content = content;
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The licence plugin's presentation half.
#[derive(Debug, Default)]
pub struct LicencePresentation;

impl PluginPresentation for LicencePresentation {
    fn name(&self) -> &'static str {
        "licence"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "LIC",
            tint: 0x00b4_8a1a,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: LicenceView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };

        let mut lines = Vec::new();
        if view.licences.is_empty() {
            lines.push("No licence recognised".to_owned());
        } else {
            lines.push(format!("Licence: {}", view.licences.join(", ")));
            if view.licences.len() > 1 {
                lines.push("Holds more than one licence".to_owned());
            }
        }

        match (&view.holder, &view.years) {
            (Some(holder), Some(years)) => lines.push(format!("Copyright {years} {holder}")),
            (Some(holder), None) => lines.push(format!("Copyright holder: {holder}")),
            (None, Some(years)) => lines.push(format!("Copyright year(s): {years}")),
            (None, None) => lines.push("No copyright holder found".to_owned()),
        }

        if !view.licences.is_empty() {
            lines.push(if view.differs_from_canonical {
                "Text differs from the canonical wording".to_owned()
            } else {
                "Text matches the canonical wording".to_owned()
            });
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LicenceCore, LicencePresentation, LicenceView, copyright_notice, normalize, parse,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    const MIT: &str = "MIT License\n\n\
        Copyright (c) 2022 Contoso Robotics, Inc.\n\n\
        Permission is hereby granted, free of charge, to any person obtaining a copy \
        of this software and associated documentation files (the \"Software\"), to deal \
        in the Software without restriction, including without limitation the rights \
        to use, copy, modify, merge, publish, distribute, sublicense, and/or sell \
        copies of the Software, and to permit persons to whom the Software is \
        furnished to do so, subject to the following conditions:\n\n\
        The above copyright notice and this permission notice shall be included in all \
        copies or substantial portions of the Software.\n\n\
        THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR \
        IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, \
        FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE \
        AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER \
        LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, \
        OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE \
        SOFTWARE.\n";

    #[test]
    fn sniffs_an_unmodified_mit_licence() {
        assert!(LicenceCore.sniff(MIT.as_bytes()));
    }

    #[test]
    fn does_not_claim_a_notice_file_that_is_not_a_licence() {
        let notice = "This product includes software developed by third parties.\n\n\
            Component: quill-render\nUsed for rendering vector icons in the toolbar.\n";
        assert!(!LicenceCore.sniff(notice.as_bytes()));
        assert!(!LicenceCore.sniff(b""));
        assert!(!LicenceCore.sniff(&[0xff, 0xfe, 0x00]));
    }

    #[test]
    fn reads_the_holder_and_years() {
        let view = parse(MIT);
        assert_eq!(view.licences, vec!["MIT".to_owned()]);
        assert_eq!(view.years.as_deref(), Some("2022"));
        assert_eq!(view.holder.as_deref(), Some("Contoso Robotics, Inc."));
        assert!(!view.differs_from_canonical);
    }

    #[test]
    fn reads_a_year_range_and_a_bare_copyright_year() {
        let (years, holder) = super::split_years_and_holder("2019-2024 Ridgeline Systems");
        assert_eq!(years.as_deref(), Some("2019-2024"));
        assert_eq!(holder.as_deref(), Some("Ridgeline Systems"));
    }

    #[test]
    fn flags_a_body_whose_wording_was_edited() {
        let edited = MIT.replace(
            "IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE",
            "IN NO EVENT SHALL ACME CORP BE LIABLE",
        );
        let view = parse(&edited);
        assert_eq!(view.licences, vec!["MIT".to_owned()]);
        assert!(view.differs_from_canonical);
    }

    #[test]
    fn a_three_clause_bsd_licence_is_not_also_reported_as_two_clause() {
        let bsd3 = "Copyright (c) 2020 Example\n\n\
            Redistribution and use in source and binary forms, with or without \
            modification, are permitted provided that the following conditions are met:\n\n\
            1. Redistributions of source code must retain the above copyright notice, \
            this list of conditions and the following disclaimer.\n\n\
            2. Redistributions in binary form must reproduce the above copyright \
            notice, this list of conditions and the following disclaimer in the \
            documentation and/or other materials provided with the distribution.\n\n\
            3. Neither the name of the copyright holder nor the names of its \
            contributors may be used to endorse or promote products derived from this \
            software without specific prior written permission.\n\n\
            THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDER AND CONTRIBUTORS \"AS \
            IS\" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, \
            THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR \
            PURPOSE ARE DISCLAIMED.\n";
        let view = parse(bsd3);
        assert_eq!(view.licences, vec!["BSD-3-Clause".to_owned()]);
    }

    #[test]
    fn skips_the_free_software_foundations_own_notice_on_the_licence_document() {
        let text = "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007\n\n\
            Copyright (C) 2007 Free Software Foundation, Inc. <https://fsf.org/>\n\
            Everyone is permitted to copy and distribute verbatim copies\n\
            of this license document, but changing it is not allowed.\n\n\
            Copyright (C) 2019 Aurora Fieldworks Cooperative\n\
            This program is free software: you can redistribute it and/or modify\n";
        let rest = copyright_notice(text).unwrap();
        assert!(rest.trim_start().starts_with("(C) 2019"));
    }

    #[test]
    fn a_malformed_or_truncated_body_does_not_panic() {
        assert!(!LicenceCore.sniff(b"Copy"));
        let view = parse("Copyright");
        assert!(view.holder.is_none());
        assert!(view.years.is_none());
        assert!(view.licences.is_empty());

        // Cut after the sniff marker but before the canonical closing
        // paragraphs: still recognised as MIT, correctly flagged as
        // differing from the canonical wording rather than panicking on
        // the missing tail.
        let cut_mid_marker = &MIT[..MIT.len() / 2];
        let view = parse(cut_mid_marker);
        assert_eq!(view.licences, vec!["MIT".to_owned()]);
        assert!(view.differs_from_canonical);
    }

    #[test]
    fn normalize_collapses_whitespace_including_line_breaks() {
        assert_eq!(normalize("a   b\n\nc\td"), "a b c d");
    }

    #[test]
    fn presents_a_licence_and_falls_back_when_none_is_recognised() {
        let data = serde_json::to_value(parse(MIT)).unwrap();
        let lines = LicencePresentation.present(&data);
        assert_eq!(lines[0], "Licence: MIT");
        assert!(lines.contains(&"Text matches the canonical wording".to_owned()));

        let empty = serde_json::to_value(LicenceView {
            licences: Vec::new(),
            holder: None,
            years: None,
            differs_from_canonical: false,
            content: String::new(),
            truncated: false,
        })
        .unwrap();
        let lines = LicencePresentation.present(&empty);
        assert_eq!(lines[0], "No licence recognised");
        assert!(lines.contains(&"No copyright holder found".to_owned()));
    }

    #[test]
    fn the_dual_licence_fixture_reports_both_and_the_notice_fixture_reports_neither() {
        let dual = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/licence/LICENSE");
        let data = LicenceCore.view(&dual).unwrap();
        let view: LicenceView = serde_json::from_value(data).unwrap();
        assert_eq!(
            view.licences,
            vec!["MIT".to_owned(), "BSD-3-Clause".to_owned()]
        );
        assert!(!view.differs_from_canonical);

        let notice = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/licence/NOTICE");
        assert!(!LicenceCore.sniff(&std::fs::read(&notice).unwrap()));
    }

    #[test]
    fn the_mit_and_apache_and_gpl_fixtures_each_fill_holder_years_and_canonical() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/licence");

        let mit: LicenceView =
            serde_json::from_value(LicenceCore.view(&root.join("LICENSE-MIT")).unwrap()).unwrap();
        assert_eq!(mit.licences, vec!["MIT".to_owned()]);
        assert!(mit.holder.is_some());
        assert!(mit.years.is_some());
        assert!(!mit.differs_from_canonical);

        let apache: LicenceView =
            serde_json::from_value(LicenceCore.view(&root.join("LICENSE-APACHE")).unwrap())
                .unwrap();
        assert_eq!(apache.licences, vec!["Apache-2.0".to_owned()]);
        assert!(!apache.differs_from_canonical);

        let gpl: LicenceView =
            serde_json::from_value(LicenceCore.view(&root.join("COPYING")).unwrap()).unwrap();
        assert_eq!(gpl.licences, vec!["GPL-3.0".to_owned()]);
        assert_eq!(gpl.holder.as_deref(), Some("Aurora Fieldworks Cooperative"));
        assert!(!gpl.differs_from_canonical);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&LicenceCore),
            plugin_api::PluginPresentation::extensions(&LicencePresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
