# Format Support

## Implemented in the first prototype

- TXT: plain text with UTF-8, GB18030/GBK-compatible decoding, and Big5 fallback.
- Markdown: currently rendered as plain text; Markdown semantic rendering is planned.

## Planned

- EPUB 2/3: parse container, OPF metadata, spine order, XHTML content, navigation, and cover.
- FB2: parse XML book metadata and body sections.
- HTML: extract readable text and headings.
- MOBI/AZW3: evaluate a maintained parser or a safe conversion integration; do not promise native support until validated.
- PDF: optional text extraction; preserve PDF page semantics separately from reflowable novels.

Every adapter should produce the same normalized document model and preserve source locations for progress and annotations.
