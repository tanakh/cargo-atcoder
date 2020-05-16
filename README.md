# cargo-atcoder

![](https://github.com/tanakh/cargo-atcoder/workflows/Rust/badge.svg) [![Join the chat at https://gitter.im/tanakh/cargo-atcoder](https://badges.gitter.im/tanakh/cargo-atcoder.svg)](https://gitter.im/tanakh/cargo-atcoder?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

Cargo subcommand for AtCoder

# 使い方

## インストール

```
$ cargo install cargo-atcoder
```

## ログイン

```
$ cargo atcoder login
```

でAtCoderにログインします。httpのセッションを保存します。ユーザー名とパスワードは保存しないので安心して下さい。`clear-session`コマンドでセッション情報を消せます。

## パッケージ作成

まずは[ワークスペース](https://doc.rust-lang.org/stable/cargo/reference/workspaces.html)を作ります。

```console
$ echo '[workspace]' > ./Cargo.toml
$ tree
.
└── Cargo.toml

0 directories, 1 file
```

バイナリ提出を行なう場合はサイズ削減のために`profile.release`を設定しましょう。

```console
$ cat << EOF > ./Cargo.toml
[workspace]

[profile.release]
lto = true
panic = "abort"
EOF
```

そして `new` コマンドでコンテスト用の[パッケージ](https://doc.rust-lang.org/stable/cargo/appendix/glossary.html#package)を作成します。

```
$ cargo atcoder new <contest-name>
```

必ずURLに含まれるコンテスト名で作成します。

例えば、ABC152 (<https://atcoder.jp/contests/abc152>) なら、`abc152`になるので、

```console
$ cargo atcoder new abc152 --skip-warmup
       Added "abc152" to `workspace.members` at /home/me/src/local/workspace/Cargo.toml
     Created binary (application) `abc152` package
     Removed the `main.rs` in `abc152`
       Added 6 `bin`(s) to `abc152`
    Modified `abc152` successfully
    Skipping warming up
```

これで`abc152`というパッケージが作られます。

```console
$ tree
.
├── abc152
│   ├── Cargo.toml
│   └── src
│       └── bin
│           ├── a.rs
│           ├── b.rs
│           ├── c.rs
│           ├── d.rs
│           ├── e.rs
│           └── f.rs
└── Cargo.toml

3 directories, 8 files
```

ソースファイルは

1. コンテストが始まっていて参加している場合、問題一覧
2. コンテストのトップページの配点表

から得られた問題のアルファベットに従い作成されます。
開始前かつ配点表がトップページに無いコンテストでは名前を`--problems`で指定してください。

```console
$ cargo atcoder new <contest-name> --problems {a..f}
```

以前のcargo-atcdoerで作成したパッケージ達は`cargo atcoder migrate`で一つのワークスペースに統合することができます。

```console
$ tree
.
├── agc001
│   ├── Cargo.toml
│   └── src
│       └── bin
│           ├── a.rs
│           ├── b.rs
│           ├── c.rs
│           ├── d.rs
│           ├── e.rs
│           └── f.rs
└── agc002
    ├── Cargo.toml
    └── src
        └── bin
            ├── a.rs
            ├── b.rs
            ├── c.rs
            ├── d.rs
            ├── e.rs
            └── f.rs

6 directories, 14 files
$ cargo atcoder migrate .
       Found `/home/me/src/local/workspace/agc001/Cargo.toml`
       Found `/home/me/src/local/workspace/agc002/Cargo.toml`
Found 2 workspace(s). Proceed? yes
       Wrote `/home/me/src/local/workspace/Cargo.toml`
       Wrote `/home/me/src/local/workspace/agc001/Cargo.toml`
       Wrote `/home/me/src/local/workspace/agc002/Cargo.toml`
$ cargo metadata --format-version 1 --no-deps | jq -r '.packages[] | .targets[] | .name' | sort
agc001-a
agc001-b
agc001-c
agc001-d
agc001-e
agc001-f
agc002-a
agc002-b
agc002-c
agc002-d
agc002-e
agc002-f
```

## 解答サブミット

作成したパッケージのディレクトリの中で、`submit`コマンドを実行すると解答をサブミットできます。

```console
$ cargo atcoder submit <problem-id>
```

カレントディレクトリがワークスペース直下の場合`-p`で、ワークスペースの外なら`--manifest-path`でパッケージを指定できます。

`problem-id`は、例えばABCなら`a`, `b`, `c`, `d`, `e`, `f`です。

サブミット前に、問題文中のテストケースでテストを行い、全て正解した場合のみサブミットを行います。オプションで強制的にサブミットしたり、サブミット前のテスト自体のスキップもできます。

`--bin` オプションを付けると、ソースコードではなく、バイナリを送りつけます。静的リンクしたバイナリを送りつけるので、お好きな処理系と、お好きなcrateが使えます。

設定ファイルで、デフォルトでバイナリを送る設定にしたり、target tripleを設定したりできます。

[UPX](https://upx.github.io/)がインストールされていれば、自動的に使って圧縮します。インストールされていても使わない設定にもできます。

実行例：

```
$ cargo atcoder submit a --bin
```

![cargo-atcoder-submit](doc/img/cargo-atcoder-submit.gif)

デフォルトでは、なるべくジャッジの環境によらずに動くように、ターゲットとして `x86_64-unknown-linux-musl` を利用するようになっています。インストールされていない場合は、

```
$ rustup target add x86_64-unknown-linux-musl
```

でインストール出来ます。

## その他コマンド

### `cargo atcoder status`

自分のサブミット状況を適当にフェッチして表示します。リアルタイム更新されます。

![cargo-atcoder-submit](doc/img/cargo-atcoder-status.gif)


### `cargo atcoder test`

テストケースの実行に特化したコマンドです。テストケースの指定や、verboseな実行ができたりします。

```
$ cargo atcoder test <problem-id>
```

`problem-id`の他に何も指定しなければ、問題文のページから入力例を自動的に取得して、全てに対してテストを行います。

```
$ cargo atcoder test <problem-id> [case-num]...
```

`case-num` には、`1`、`2`、`3` などの入力例の番号を1つまたは複数指定できます。`-v` を付けるとなんか少し多めに情報が出るかも知れません。

```
$ cargo atcoder test <problem-id> --custom
```

`--custom` を付けると、標準入力から入力するモードになります。

### `cargo atcoder gen-binary`

```
$ cargo atcoder gen-binary <problem-id>
```

`problem-id` のRustのコードとしてサブミットできるバイナリを生成します。`submit`の`--bin`オプションで生成する物と同じです。

### `cargo atcoder result`

```
$ cargo atcoder result [FLAGS] <submission-id>
```

サブミット結果の詳細を表示します。ACじゃなかった場合は結果の内訳を表示します。全テストケースが開示されている場合は全テストケースに対する結果を取得して表示します。

## 設定ファイル

`~/.config/cargo-atcoder.toml` に設定ファイルが生成されます。適当にいじって下さい（そのうち説明を書く）。

## macOS 環境の場合

設定ファイルは `~/Library/Preferences/cargo-atcoder.toml` に生成されます。

`x86_64-unknown-linux-musl` 向けのコンパイルを面倒無く実行するため、`[atcoder]` テーブル内で `use_cross = true` を指定するのがおすすめです。`use_cross` を有効化することで、[rust-embedded/cross](https://github.com/rust-embedded/cross) を使用したコンパイルを行うようになります。Docker が必要になるので注意してください。
crossのインストールもお忘れなく。
```
$ cargo install cross
```

また、実行バイナリを軽量化するために使われる `strip` コマンドが、macOS に最初から入っているものだとうまくいかないため、**GNU版**の `strip` を導入するのもおすすめです。Homebrewであれば以下を実行すればインストールすることができます。

```
$ brew install binutils
```

標準では `/usr/local/opt/binutils/bin` の中にインストールされます。
ここにPATHを通すか、あるいは `cargo-atcoder.toml` の `[atcoder]` テーブル内に以下のようにGNU版 `strip` の絶対パスを指定すればOKです。

```
[atcoder]
strip_path = "/usr/local/opt/binutils/bin/strip"
```
