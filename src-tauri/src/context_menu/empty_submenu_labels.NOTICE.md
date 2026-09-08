# Chromium empty submenu translations

`empty_submenu_labels.json` contains only `IDS_APP_MENU_EMPTY_SUBMENU` from
Chromium **140.0.7339.41**. It includes the English source and all 81
translations declared by that version's GRD. No translations are missing,
inferred, or rewritten.

Sources, pinned to the same Chromium version:

- [Message and language declarations](https://raw.githubusercontent.com/chromium/chromium/140.0.7339.41/ui/strings/ui_strings.grd)
- Translation files: `ui/strings/` plus each `<translations><file path>` in that GRD, under
  `https://raw.githubusercontent.com/chromium/chromium/140.0.7339.41/`.
- [GRIT fingerprint](https://raw.githubusercontent.com/chromium/chromium/140.0.7339.41/tools/grit/grit/extern/FP.py)
  and [message ID algorithm](https://raw.githubusercontent.com/chromium/chromium/140.0.7339.41/tools/grit/grit/extern/tclib.py).

The English text is `(empty)` with no `meaning` attribute. GRIT computes the
translation ID from the first 64 bits of its UTF-8 MD5, clearing the high bit:
`int(md5(b"(empty)").hexdigest()[:16], 16) & 0x7fffffffffffffff`.
The resulting ID is **2190355936436201913**. Each translated value is the
XML-decoded text of exactly that ID, preserving punctuation and Unicode.

Keys retain GRD language identifiers. In particular, `he` reads the `iw`
translation file, `no` is emitted as `nb.pak`, and the English source `en` is
emitted as `en-US.pak`. Those are upstream locale mappings, not additional
translations. Pseudolocales are generated upstream and have no XTB source;
they are not entries in this file.

Verification anchors:

| Language | Value | Source |
| --- | --- | --- |
| en | `(empty)` | `ui_strings.grd`, named message |
| en-GB | `(empty)` | `translations/ui_strings_en-GB.xtb`, ID above |
| zh-TW | `(空白)` | `translations/ui_strings_zh-TW.xtb`, ID above |
| ja | `(なし)` | `translations/ui_strings_ja.xtb`, ID above |

## License

The extracted data is distributed under Chromium's BSD-style license below,
reproduced from the [version-pinned LICENSE](https://raw.githubusercontent.com/chromium/chromium/140.0.7339.41/LICENSE).
Retain this notice with source distributions and reproduce it in binary
distribution documentation or accompanying materials.

```text
// Copyright 2015 The Chromium Authors
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//    * Redistributions of source code must retain the above copyright
// notice, this list of conditions and the following disclaimer.
//    * Redistributions in binary form must reproduce the above
// copyright notice, this list of conditions and the following disclaimer
// in the documentation and/or other materials provided with the
// distribution.
//    * Neither the name of Google LLC nor the names of its
// contributors may be used to endorse or promote products derived from
// this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```
