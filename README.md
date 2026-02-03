# srvplz

Minimal CLI static HTTP server for the current directory (or a provided directory). Written in Rust.

## Usage

```bash
srvplz [directory]
```

- Serves files on port 8000 by default, retrying the next port if busy (up to 25 retries).
- No directory listings. Directories are served only via `index.html` if present.
- Directories will be hosted **without a trailing slash** (e.g., `/about` instead of `/about/`).
- Requests are logged to stdout.

## Installation

You can install `srvplz` using Cargo:

```bash
cargo install srvplz
```

## License

MIT
