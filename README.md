![](https://github.com/tanakh/cargo-atcoder/workflows/Rust/badge.svg)

# cargo-atcoder
Cargo subcommand for AtCoder

# 使い方

## インストール

```
$ cargo install --git https://github.com/tanakh/cargo-atcoder.git
```

## ログイン

```
$ cargo atcoder login
```

でAtCoderにログインします。httpのセッションを保存します。ユーザー名とパスワードは保存しないので安心して下さい。`clear-session`コマンドでセッション情報を消せます。

## プロジェクト作成

`new` コマンドでコンテスト用のプロジェクトファイルを作成します。

```
$ cargo atcoder new <contest-name>
```

必ずURLに含まれるコンテスト名で作成します。

例えば、ABC152 (<https://atcoder.jp/contests/abc152>) なら、`abc152`になるので、

```console
$ cargo atcoder new abc152
     Created binary (application) `abc152` package
```

これで`abc152`というディレクトリが作られて、そこにcargoのプロジェクトが作られます。

```console
$ tree ./abc152
./abc152
├── Cargo.toml
└── src
    └── bin
        ├── a.rs
        ├── b.rs
        ├── c.rs
        ├── d.rs
        ├── e.rs
        └── f.rs

2 directories, 7 files
```

ソースファイルは

1. コンテストが始まっていて参加している場合、問題一覧
2. ratedの場合、コンテストのトップページの配点表

から得られた問題のアルファベットに従い作成されます。
開始前のunratedなコンテストではfile stemを`-b`, `--bins`で指定してください。

```
$ cargo atcoder new <contest-name> -b {a..f}
```

## 解答サブミット

作成したプロジェクトのディレクトリの中で、`submit`コマンドを実行すると解答をサブミットできます。

```
$ cargo atcoder submit <problem-id>
```

`problem-id`は、URLの末尾に含まれるものを指定します（例えば、<https://atcoder.jp/contests/abc152/tasks/abc152_a> なら、`a`）。


サブミット前に、問題文中のテストケースでテストを行い、全て正解した場合のみサブミットを行います。オプションで強制的にサブミットしたり、サブミット前のテスト自体のスキップもできます。

`--bin` オプションを付けると、ソースコードではなく、バイナリを送りつけます。静的リンクしたバイナリを送りつけるので、お好きな処理系と、お好きなcrateが使えます。

設定ファイルで、デフォルトでバイナリを送る設定にしたり、target tripleを設定したりできます。

```
$ cargo atcoder submit a --bin
```

実行例：

![cargo-atcoder-submit](doc/img/cargo-atcoder-submit.gif)

## その他コマンド

### `cargo atcoder status`

自分のサブミット状況を適当にフェッチして表示します。リアルタイム更新されます。

![cargo-atcoder-submit](doc/img/cargo-atcoder-status.gif)


### `cargo atcoder test`

テストケースの実行に特化したコマンドです。テストケースの指定や、verboseな実行ができたりします。

## 設定ファイル

`~/.config/cargo-atcoder.toml` に設定ファイルが生成されます。適当にいじって下さい（そのうち説明を書く）。
