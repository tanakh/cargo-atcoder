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

```
$ cargo atcoder new abc152
```

これで`abc152`というディレクトリが作られて、そこにcargoのプロジェクトが作られます。

問題数を指定することもできます（デフォルトでは6）。指定した問題数分のソースファイルが作成されます。

```
$ cargo atcoder new abc152 6
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
