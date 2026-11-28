# E06 multilingual PaddleOCR acceptance

The isolated PP-OCRv5 Latin recognizer is retained because the official model documentation lists Vietnamese among its supported languages. The real synthetic acceptance requires the exact accented Vietnamese phrase `thanh toán`, the exact amount `125000`, valid bounded geometry, pinned runtime/model metadata, offline inference and a human-review result. It deliberately does not require every surrounding synthetic word to be exact because multilingual OCR can vary on individual glyph combinations; no normalization or fuzzy correction is used to manufacture a pass.

PP-OCRv3 Latin remains an independent comparison job with the same exact accented phrase/amount acceptance. Both execute with the network disabled after model image build. This is compatibility evidence only, not a production accuracy claim; the release benchmark in NFR-009 remains separate.
