#!/usr/bin/env python3
"""
TidyDog SA1 content-reading sidecar.

Protocol (C-A):
  stdin:  one line of JSON  {"path": "...", "max_chars": 4096}
  stdout: one line of JSON  {"text": "...", "pages": <int|null>, "truncated": <bool>, "error": <str|null>}

Safety constraint N5: read-only — this script never modifies, moves, renames,
or deletes the source file.
"""

import json
import sys
import os


def _error_response(message: str) -> dict:
    return {"text": "", "pages": None, "truncated": False, "error": message}


def _ok_response(text: str, pages, truncated: bool) -> dict:
    return {"text": text, "pages": pages, "truncated": truncated, "error": None}


def _truncate(text: str, max_chars: int):
    """Return (possibly-truncated text, was_truncated)."""
    if len(text) <= max_chars:
        return text, False
    return text[:max_chars], True


def read_text_file(path: str, max_chars: int) -> dict:
    """Read a plain-text file (txt / md / rst / etc.)."""
    try:
        with open(path, "rb") as fh:
            raw = fh.read()
    except OSError as exc:
        return _error_response(f"cannot read file: {exc}")

    # Try UTF-8 first, fall back to latin-1 (never fails).
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        text = raw.decode("latin-1")

    text, truncated = _truncate(text, max_chars)
    return _ok_response(text, None, truncated)


def read_pdf_file(path: str, max_chars: int) -> dict:
    """Extract text from a PDF using pypdf (pdfplumber as optional fallback)."""

    # ------------------------------------------------------------------ pypdf
    try:
        import pypdf  # noqa: PLC0415
    except ImportError:
        pypdf = None  # type: ignore[assignment]

    if pypdf is not None:
        try:
            reader = pypdf.PdfReader(path)
            total_pages = len(reader.pages)
            chunks: list[str] = []
            chars_collected = 0

            for page in reader.pages:
                if chars_collected >= max_chars:
                    break
                page_text = page.extract_text() or ""
                remaining = max_chars - chars_collected
                chunks.append(page_text[:remaining])
                chars_collected += len(page_text[:remaining])

            full_text = "".join(chunks)
            # Strip common PDF artifacts (leading/trailing whitespace per chunk).
            full_text = full_text.strip()

            if not full_text:
                # Fall through to pdfplumber if available, otherwise error.
                pass
            else:
                truncated = chars_collected >= max_chars and sum(
                    len(p.extract_text() or "") for p in reader.pages
                ) > max_chars
                # Re-check truncation accurately.
                all_text_len = sum(
                    len(p.extract_text() or "") for p in reader.pages
                )
                truncated = all_text_len > max_chars
                return _ok_response(full_text, total_pages, truncated)

        except pypdf.errors.PdfStreamError as exc:
            return _error_response(f"broken or encrypted PDF: {exc}")
        except pypdf.errors.PdfReadError as exc:
            return _error_response(f"cannot read PDF: {exc}")
        except Exception as exc:  # noqa: BLE001
            return _error_response(f"PDF error: {exc}")

    # -------------------------------------------------------------- pdfplumber
    try:
        import pdfplumber  # noqa: PLC0415
    except ImportError:
        pdfplumber = None  # type: ignore[assignment]

    if pdfplumber is not None:
        try:
            with pdfplumber.open(path) as pdf:
                total_pages = len(pdf.pages)
                chunks = []
                chars_collected = 0

                for page in pdf.pages:
                    if chars_collected >= max_chars:
                        break
                    page_text = page.extract_text() or ""
                    remaining = max_chars - chars_collected
                    chunks.append(page_text[:remaining])
                    chars_collected += len(page_text[:remaining])

                full_text = "".join(chunks).strip()

            if not full_text:
                return _error_response("no extractable text (image-only PDF?)")

            # Estimate truncation: if we stopped early or last page was cut.
            truncated = chars_collected >= max_chars
            return _ok_response(full_text, total_pages, truncated)

        except Exception as exc:  # noqa: BLE001
            return _error_response(f"PDF error (pdfplumber): {exc}")

    # Neither library available.
    if pypdf is None and pdfplumber is None:
        return _error_response(
            "no PDF library available (install pypdf)"
        )

    # pypdf was available but returned no text.
    return _error_response("no extractable text (image-only PDF?)")


TEXT_EXTENSIONS = {".txt", ".md", ".rst", ".text", ".markdown"}
PDF_EXTENSION = ".pdf"


def process(path: str, max_chars: int) -> dict:
    """Dispatch to the correct reader based on file extension."""
    if not os.path.exists(path):
        return _error_response(f"file not found: {path}")
    if not os.path.isfile(path):
        return _error_response(f"not a regular file: {path}")
    if not os.access(path, os.R_OK):
        return _error_response(f"permission denied: {path}")

    ext = os.path.splitext(path)[1].lower()

    if ext == PDF_EXTENSION:
        return read_pdf_file(path, max_chars)
    elif ext in TEXT_EXTENSIONS:
        return read_text_file(path, max_chars)
    else:
        # Unknown extension: try as UTF-8 text.
        return read_text_file(path, max_chars)


def main() -> None:
    try:
        line = sys.stdin.readline()
        if not line:
            result = _error_response("empty stdin")
            sys.stdout.write(json.dumps(result) + "\n")
            return

        try:
            request = json.loads(line)
        except json.JSONDecodeError as exc:
            result = _error_response(f"invalid JSON input: {exc}")
            sys.stdout.write(json.dumps(result) + "\n")
            return

        path = request.get("path", "")
        max_chars = request.get("max_chars", 4096)

        if not path:
            result = _error_response("missing 'path' in request")
            sys.stdout.write(json.dumps(result) + "\n")
            return

        if not isinstance(max_chars, int) or max_chars <= 0:
            result = _error_response("'max_chars' must be a positive integer")
            sys.stdout.write(json.dumps(result) + "\n")
            return

        result = process(path, max_chars)

    except Exception as exc:  # noqa: BLE001 — last-resort safety net
        result = _error_response(f"unexpected error: {exc}")

    sys.stdout.write(json.dumps(result) + "\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
