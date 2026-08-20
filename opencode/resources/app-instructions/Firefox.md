## Firefox Computer Use

Firefox accessibility is useful for discovering window/app state and some document structure, but web content still needs caution. Prefer semantic tree reads for high-level focus and role discovery, and use physical input fallback for interaction until element routing is verified.

Treat text entry with care: focus first, then type. Do not assume a semantic value-write path exists for arbitrary web content.

