# WebVTT test fixtures

`settings-multiple.vtt`, `timings-omitted-hours.vtt`, `comment-in-cue-text.vtt`, `header-space.vtt`, and `timings-60.vtt` come from the [Web Platform Tests WebVTT file parsing fixtures](https://github.com/web-platform-tests/wpt/tree/831ff48a8af316bd98c5b79d2cb61c84764de41d/webvtt/parsing/file-parsing/tests/support). Their 3-Clause BSD license is preserved in `LICENSE.md`. The `timings-60.vtt` fixture contains deliberately invalid cues; Muxcraft reports an error instead of silently dropping them.

Keep these files byte-for-byte as upstream provides them. In particular, `header-space.vtt` intentionally has a line containing one space after the `WEBVTT` header.

`w3c-features.vtt` is an original, compact fixture based on the feature categories in the [W3C WebVTT introduction](https://www.w3.org/TR/2026/CRD-webvtt1-20260520/#introduction-caption): styling, regions, comments, cue identifiers, settings, voice and language tags, ruby text, and inline timestamps. Its subtitle text is original to this repository.
