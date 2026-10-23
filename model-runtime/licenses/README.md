# Supplemental dependency notices

`supplements.json` supplies an exact-version notice only when a reviewed wheel has neither license metadata nor bundled license text. Unknown versions remain build errors. The builder reads these local files, verifies their SHA-256 and retains the text, immutable source URL and hash in the pack's license inventory. It fetches no notices during build or installation.

Tokenizers 0.22.2's pinned Linux wheel omits all three license metadata fields and notice files. Its [matching source commit](https://github.com/huggingface/tokenizers/tree/f383101a26663708484cac0727792aad74f78234) declares Apache-2.0 in `tokenizers/Cargo.toml`. The unmodified [upstream LICENSE](https://raw.githubusercontent.com/huggingface/tokenizers/f383101a26663708484cac0727792aad74f78234/LICENSE) is retained here; its SHA-256 is `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4`. Git attributes preserve LF bytes on native Windows checkouts as well.

This is notice preparation, not blanket distribution approval. The release's existing inventory/license-review acceptance requirement remains.
