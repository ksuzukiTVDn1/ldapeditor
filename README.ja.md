# ldapeditor

<div align="center">

[![Crate Badge]][Crate] [![CI Badge]][CI] [![Codecov Badge]][Codecov] [![Rust Badge]][Rust] [![License Badge]][License]

[English](README.md) | **日本語**

</div>

[OpenLDAP] のエントリをキーボード操作で閲覧・編集できるターミナル UI です。LDIF ファイルは不要です。

`ldapi://` (SASL EXTERNAL) または `ldap[s]://` (Simple Bind) で接続し、SSH セッション
(さらには **bare Linux VT・シリアルコンソール** — 8 色 SGR のみで描画し、256 色や RGB は使いません)
から DIT のナビゲーション、属性値の編集、objectClass の管理をインタラクティブに行えます。

**スキーマ駆動**: 起動時に `subschemaSubentry` をパースして以下を提供します。
属性 / objectClass のあいまい検索ピッカー、MUST と SINGLE-VALUE を示す `[N]` / `[S]` バッジ、
objectClass 追加時の未設定 MUST 属性の自動プロンプト、objectClass 削除時の孤立属性検出。

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

## インストール

**[crates.io] から**:

```sh
cargo install ldapeditor
```

**ビルド済みバイナリ** — [Releases] を参照。各リリースは以下を同梱します。

| アセット | ターゲット | 備考 |
|---|---|---|
| `ldapeditor-vX.Y.Z-x86_64-linux-gnu`   | x86-64, glibc 2.34+ | RHEL 9 / Rocky 9 / Ubuntu 22.04+ / Debian 12+ |
| `ldapeditor-vX.Y.Z-aarch64-linux-gnu`  | ARM64, glibc 2.31+  | Linux ARM64 サーバ |
| `ldapeditor-vX.Y.Z-x86_64-linux-musl`  | x86-64, static      | どこでも動く (Alpine 等)。glibc / OpenSSL 不要 |
| `ldapeditor-vX.Y.Z-aarch64-linux-musl` | ARM64, static       | 同上 (ARM64) |
| `SHA256SUMS`                           | —                   | `sha256sum -c SHA256SUMS` で検証 |

**ソースからビルド** (Rust 1.85+):

```sh
# RHEL / Rocky / Fedora
sudo dnf install gcc openssl-devel pkgconf-pkg-config

# Debian / Ubuntu
sudo apt install build-essential libssl-dev pkg-config

cargo build --release
```

> **RHEL 8 / Rocky 8:** `linux-gnu` のビルド済みバイナリは OpenSSL 3 を要求します。`linux-musl` の静的バイナリを使うか、ソースからビルドしてください。

## 使い方

```
ldapeditor [--uri URI] [--bind-dn DN] [-b BASE_DN]
```

| フラグ | デフォルト | 説明 |
|---|---|---|
| `--uri` | `ldapi://%2fvar%2frun%2fslapd%2fldapi` | LDAP サーバ URI |
| `--bind-dn` | | Simple Bind の DN。パスワードはプロンプトで入力 |
| `-b` | | Base DN。省略時は `namingContexts` から選択 |

| URI | `--bind-dn` | 認証方式 |
|---|---|---|
| `ldapi://` | 未指定 | SASL EXTERNAL (Unix ソケットの peer) |
| `ldap://` / `ldaps://` | 指定 | Simple Bind (パスワードプロンプト) |
| 任意 | 未指定 | 匿名 |

パスワードは常にインタラクティブに入力します。CLI フラグでの指定は受け付けません。

### 国際化

UI は **英語** (デフォルト) と **日本語** に対応しています。ロケールは `$LC_ALL` / `$LANG`
環境変数から判定し、判定不能なら英語にフォールバックします。翻訳の追加は歓迎です — `locales/` を参照してください。

## キーバインド

<details>
<summary>キーバインド一覧を表示</summary>

**グローバル**

| キー | 動作 |
|---|---|
| `q` | 終了 |
| `Tab` | ペイン切替 |
| `Ctrl+r` / `F5` | LDAP から再読込 |
| `/` | LDAP フィルタ検索 |
| `Esc` | モーダルを閉じる / 検索終了 |

**ツリーペイン**

| キー | 動作 |
|---|---|
| `↑↓` / `jk` | 移動 |
| `→` / `l` | 展開 (子をフェッチ) |
| `←` / `h` | 折りたたみ / 親へ |
| `a` | 子エントリ作成 (ウィザード) |
| `d` | エントリ削除 |

**詳細ペイン**

| キー | 動作 |
|---|---|
| `↑↓` / `jk` | 行移動 |
| `←→` / `hl` | 列移動 |
| `e` | 値の編集 |
| `a` | 属性 / objectClass の追加 |
| `d` | 削除 |

</details>

## 開発

```sh
cargo test     # 単体テスト (スキーマパーサ、ツリー操作、孤立属性ロジック)
cargo clippy   # lint
cargo fmt      # フォーマット
```

## ライセンス

[MIT](LICENSE-MIT) または [Apache-2.0](LICENSE-APACHE) のいずれかを選択してください (デュアルライセンス)。

---

[CI]: https://github.com/ksuzukiTVDn1/ldapeditor/actions/workflows/ci.yml
[Codecov]: https://codecov.io/gh/ksuzukiTVDn1/ldapeditor
[Crate]: https://crates.io/crates/ldapeditor
[crates.io]: https://crates.io/crates/ldapeditor
[Releases]: ../../releases
[OpenLDAP]: https://www.openldap.org
[Rust]: https://www.rust-lang.org
[License]: #ライセンス

[Crate Badge]: https://img.shields.io/crates/v/ldapeditor?style=flat-square&logo=rust
[CI Badge]: https://img.shields.io/github/actions/workflow/status/ksuzukiTVDn1/ldapeditor/ci.yml?style=flat-square&logo=github&label=CI
[Codecov Badge]: https://img.shields.io/codecov/c/github/ksuzukiTVDn1/ldapeditor?style=flat-square&logo=codecov
[Rust Badge]: https://img.shields.io/badge/rust-1.85%2B-orange?style=flat-square&logo=rust
[License Badge]: https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue?style=flat-square
