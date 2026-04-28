# ldapeditor

<div align="center">

[![Crate Badge]][Crate] [![CI Badge]][CI] [![Codecov Badge]][Codecov] [![Rust Badge]][Rust] [![License Badge]][License]

**English** | [日本語](README.ja.md)

</div>

A keyboard-driven terminal UI for browsing and editing [OpenLDAP] entries — no LDIF files required.

Connects via `ldapi://` (SASL EXTERNAL) or `ldap[s]://` (Simple Bind) and lets you navigate the
DIT, edit attribute values, and manage objectClasses interactively from any SSH session —
**including bare Linux VT and serial consoles** (8-color SGR only, no 256-color or RGB).

**Schema-aware throughout**: fuzzy attribute and objectClass pickers, `[N]`/`[S]` badges for
MUST and SINGLE-VALUE, automatic prompts for unset MUST attributes on objectClass addition,
and orphan-attribute detection on objectClass deletion — all driven by `subschemaSubentry`
parsed at startup.

```
┌────────────────────┬──────────────────────────────────────┐
│       Tree         │   dn: cn=admin,dc=example,dc=com     │
│                    ├──────────────────────────────────────┤
│  ▼ dc=example,...  │   objectClass                        │
│  ├─ ▶ ou=users     │     top                              │
│  └─ ▶ ou=groups    │     person                           │
│                    │     [+ add objectClass]              │
│                    │   attributes                         │
│                    │     cn   [N] │ admin                 │
│                    │     sn   [N] │ Smith                 │
│                    │     [+ add attribute]                │
│                    │   operational  [read only]           │
└────────────────────┴──────────────────────────────────────┘
```

## Installation

**From [crates.io]**:

```sh
cargo install ldapeditor
```

**Pre-built binaries** — see [Releases]. Each release ships:

| Asset | Target | Notes |
|---|---|---|
| `ldapeditor-vX.Y.Z-x86_64-linux-gnu`   | x86-64, glibc 2.34+ | RHEL 9 / Rocky 9 / Ubuntu 22.04+ / Debian 12+ |
| `ldapeditor-vX.Y.Z-aarch64-linux-gnu`  | ARM64, glibc 2.31+  | Linux on ARM64 servers |
| `ldapeditor-vX.Y.Z-x86_64-linux-musl`  | x86-64, static      | Runs anywhere (Alpine etc.); no glibc/OpenSSL required |
| `ldapeditor-vX.Y.Z-aarch64-linux-musl` | ARM64, static       | Same, for ARM64 |
| `SHA256SUMS`                           | —                   | Verify with `sha256sum -c SHA256SUMS` |

**Build from source** (Rust 1.85+):

```sh
# RHEL / Rocky / Fedora
sudo dnf install gcc openssl-devel pkgconf-pkg-config

# Debian / Ubuntu
sudo apt install build-essential libssl-dev pkg-config

cargo build --release
```

> **RHEL 8 / Rocky 8:** The `linux-gnu` pre-built binary requires OpenSSL 3. Use the `linux-musl` static binary or build from source.

## Usage

```
ldapeditor [--uri URI] [--bind-dn DN] [-b BASE_DN]
```

| Flag | Default | Description |
|---|---|---|
| `--uri` | `ldapi://%2fvar%2frun%2fslapd%2fldapi` | LDAP server URI |
| `--bind-dn` | | DN for Simple Bind; prompts for password |
| `-b` | | Base DN; select from `namingContexts` if omitted |

| URI | `--bind-dn` | Auth method |
|---|---|---|
| `ldapi://` | not set | SASL EXTERNAL (Unix socket peer) |
| `ldap://` / `ldaps://` | set | Simple Bind (password prompted) |
| any | not set | Anonymous |

Passwords are always entered interactively — never via CLI flags.

### Localization

UI is available in **English** (default) and **Japanese**. The locale is detected from the
`$LC_ALL` / `$LANG` environment variables and falls back to English. Translation contributions
welcome — see `locales/`.

## Key Bindings

<details>
<summary>Show key bindings</summary>

**Global**

| Key | Action |
|---|---|
| `q` | Quit |
| `Tab` | Switch pane |
| `Ctrl+r` / `F5` | Reload from LDAP |
| `/` | LDAP filter search |
| `Esc` | Close modal / exit search |

**Tree pane**

| Key | Action |
|---|---|
| `↑↓` / `jk` | Navigate |
| `→` / `l` | Expand (fetches children) |
| `←` / `h` | Collapse / go to parent |
| `a` | Create child entry (wizard) |
| `d` | Delete entry |

**Detail pane**

| Key | Action |
|---|---|
| `↑↓` / `jk` | Move row |
| `←→` / `hl` | Move column |
| `e` | Edit value |
| `a` | Add attribute / objectClass |
| `d` | Delete |

</details>

## Development

```sh
cargo test     # unit tests (schema parser, tree navigation, orphan-attribute logic)
cargo clippy   # lint
cargo fmt      # format
```

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

---

[CI]: https://github.com/ksuzukiTVDn1/ldapeditor/actions/workflows/ci.yml
[Codecov]: https://codecov.io/gh/ksuzukiTVDn1/ldapeditor
[Crate]: https://crates.io/crates/ldapeditor
[crates.io]: https://crates.io/crates/ldapeditor
[Releases]: ../../releases
[OpenLDAP]: https://www.openldap.org
[Rust]: https://www.rust-lang.org
[License]: #license

[Crate Badge]: https://img.shields.io/crates/v/ldapeditor?style=flat-square&logo=rust
[CI Badge]: https://img.shields.io/github/actions/workflow/status/ksuzukiTVDn1/ldapeditor/ci.yml?style=flat-square&logo=github&label=CI
[Codecov Badge]: https://img.shields.io/codecov/c/github/ksuzukiTVDn1/ldapeditor?style=flat-square&logo=codecov
[Rust Badge]: https://img.shields.io/badge/rust-1.85%2B-orange?style=flat-square&logo=rust
[License Badge]: https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square
