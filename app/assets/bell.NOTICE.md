# Native terminal bell fallback

Source: `tabby-terminal/src/bell.ogg` from Eugeny/tabby commit
`14e2d60b9b6dee84a53c37f05eefeb803787de04`. Retain its attribution and
[repository license](../../LICENSE).

Source Ogg SHA-256:
`7fcf3bc2f78ece87c8bb4f0728385b607b812afc4f703ba17503d82c3a15922c`.

`bell.wav` contains that sound decoded by the pinned Electron 38.8.6 runtime:
`AudioContext({ sampleRate: 48000 }).decodeAudioData(sourceBytes)`, channel 0,
converted to signed 16-bit little-endian PCM with
`Math.max(-32768, Math.min(32767, Math.round(sample * 32768)))`. Retain all 15,936 decoded
frames, including the decoder's tail. A capture from the actual reference
HTMLMediaElement's audio source matches these samples. The RIFF/WAVE file uses
one 16-byte PCM format chunk and one data chunk, mono at 48,000 Hz.

PCM data SHA-256:
`f4893d606c1cde6f519a72abc0296b57a0036024f8b733d864a1ec48c6a2276c`.

The Tauri-only import shim selects this PCM resource when the WebView reports
no support for `audio/ogg; codecs="vorbis"`. Ogg-capable hosts keep the original
bytes. Both resources must remain same-origin assets under the existing content
policy. WebKit documents Ogg support starting with macOS Sequoia 15.4:
[WebKit Features in Safari 18.4](https://webkit.org/blog/16574/webkit-features-in-safari-18-4/).
Actual older-macOS desktop acceptance remains required.
