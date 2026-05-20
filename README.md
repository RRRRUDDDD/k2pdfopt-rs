# k2pdfopt-rs

[![License: AGPL-3.0-or-later](https://img.shields.io/badge/License-AGPL%20v3-blue)](LICENSE)
[![Rust Edition 2021](https://img.shields.io/badge/Rust-2021-orange)](Cargo.toml)

**把电脑上的 PDF 变成 Kindle / Kobo 等电子书阅读器上能舒服阅读的小屏 PDF。**

这是 [k2pdfopt](https://www.willus.com/k2pdfopt/) v2.55（C 版本）的 Rust 重写。一个命令行工具，没有图形界面，没有安装包，编译出来就是一个独立可执行文件。

主要功能：

- 把 A4 / Letter 大小的 PDF 重新排版到 6 英寸电子墨水屏（Kindle Voyage、Paperwhite、Kobo 等）
- 自动识别双列学术论文并合理拆分
- 文本重排（reflow），不再"一行小字配大量空白"
- 多语言 OCR（中文、英文、日韩等），生成可复制选中的 PDF
- 30 个预置设备 profile，开箱即用

---

## 目录

- [安装](#安装)
- [5 分钟上手](#5-分钟上手)
- [常见使用场景](#常见使用场景)
- [所有常用参数](#所有常用参数)
- [支持的设备 profile](#支持的设备-profile)
- [OCR 完整指南](#ocr-完整指南)
- [常见问题](#常见问题)
- [Shell 自动补全](#shell-自动补全)
- [退出码](#退出码)
- [License](#license)

---

## 安装

### 第 1 步：装 Rust 工具链

如果你还没装：访问 https://rustup.rs 按提示安装。安装完毕后在新终端里验证：

```sh
rustc --version    # 应输出 rustc 1.75.0 或更高
cargo --version
```

### 第 2 步：装运行时依赖

| 工具 | 是否必需 | 用途 |
|------|----------|------|
| **mutool**（mupdf-tools 包里） | **必需** | 用来读取 / 渲染 PDF |
| **tesseract** + 语言包 | 仅用 `--ocr` 时必需 | OCR 引擎 |

各平台安装命令：

**Windows（推荐 Scoop）**

```powershell
scoop install mupdf tesseract
# 装中文 OCR 包
scoop bucket add extras
scoop install tesseract-languages
```

**macOS**

```sh
brew install mupdf-tools tesseract
brew install tesseract-lang   # 装全部语言包
```

**Ubuntu / Debian**

```sh
sudo apt update
sudo apt install mupdf-tools tesseract-ocr
# 装中文简体 + 英文（按需更换）
sudo apt install tesseract-ocr-chi-sim tesseract-ocr-eng
```

**Arch / Manjaro**

```sh
sudo pacman -S mupdf-tools tesseract tesseract-data-eng tesseract-data-chi_sim
```

装完后验证：

```sh
mutool -v          # 应输出 mutool version 1.x.x
tesseract --list-langs    # 应能列出已装语言
```

### 第 3 步：编译 k2pdfopt-rs

```sh
git clone https://github.com/RRRRUDDDD/k2pdfopt-rs.git
cd k2pdfopt-rs
cargo build --release
```

编译完成后，可执行文件在 `target/release/k2pdfopt`（Linux/macOS）或 `target/release/k2pdfopt.exe`（Windows）。

可以把它复制到 PATH 里方便调用：

```sh
# Linux / macOS
sudo cp target/release/k2pdfopt /usr/local/bin/

# macOS（用户级，无需 sudo）
mkdir -p ~/.local/bin
cp target/release/k2pdfopt ~/.local/bin/

# Windows PowerShell
copy target\release\k2pdfopt.exe C:\Users\$env:USERNAME\bin\
```

### 第 4 步：验证

```sh
k2pdfopt --version
k2pdfopt --help
```

---

## 5 分钟上手

最常见用法 —— 把一份 PDF 转成 Kindle Paperwhite 友好版：

```sh
k2pdfopt --dev kpw input.pdf
```

输出文件叫 `input_k2opt.pdf`，和原文件放在同一个目录。

就这一行命令喵～(*^▽^*)

---

## 常见使用场景

### 场景 1：转给 Kindle Paperwhite / Voyage 看

```sh
# Paperwhite（6 英寸 300dpi）
k2pdfopt --dev kpw input.pdf

# Voyage / Paperwhite 3+（6 英寸 300dpi，更清晰）
k2pdfopt --dev kv input.pdf

# Oasis 2/3（7 英寸）
k2pdfopt --dev ko2 input.pdf
```

### 场景 2：转给 Kobo 看

```sh
k2pdfopt --dev kobo input.pdf
```

### 场景 3：双列学术论文拆分阅读

PDF 是双列排版（学术论文常见），希望按"先读左列，再读右列"的顺序在小屏上看：

```sh
k2pdfopt --dev kpw --reflow force paper.pdf
```

`--reflow force` 强制启用完整文本重排，效果最好。

### 场景 4：扫描版 PDF 加 OCR（让文字可以选中复制）

```sh
# 英文扫描书
k2pdfopt --dev kpw --ocr eng scan.pdf

# 中文简体扫描书
k2pdfopt --dev kpw --ocr chi_sim scan.pdf

# 中英文混合
k2pdfopt --dev kpw --ocr "chi_sim+eng" scan.pdf

# 日英混合
k2pdfopt --dev kpw --ocr "jpn+eng" scan.pdf
```

### 场景 5：只转某些页

```sh
# 只转第 1 - 10 页
k2pdfopt --dev kpw -p 1-10 input.pdf

# 转第 1, 3, 5, 7 页
k2pdfopt --dev kpw -p 1,3,5,7 input.pdf

# 转所有偶数页
k2pdfopt --dev kpw -p even input.pdf

# 跳过封面前 5 页
k2pdfopt --dev kpw --px 1-5 input.pdf
```

### 场景 6：自定义输出文件名

```sh
# %s 会被替换成原文件名（不带扩展名）
k2pdfopt --dev kpw -o "%s_kindle.pdf" input.pdf
# 输出：input_kindle.pdf
```

### 场景 7：横屏模式（适合大开本书）

```sh
k2pdfopt --dev kpw --ls input.pdf
```

### 场景 8：批量转换一整个文件夹

```sh
# 把 ~/Downloads/papers 文件夹下所有 PDF 都转一遍
k2pdfopt --dev kpw -x ~/Downloads/papers
```

`-x` 表示转完就退出，不进入交互模式。

### 场景 9：调整裁剪边距

源 PDF 边距太大，想多裁掉一些：

```sh
# 上下左右各裁 0.5 英寸
k2pdfopt --dev kpw -m 0.5 input.pdf

# 分别设置：左 0.3, 上 0.5, 右 0.3, 下 0.5（单位英寸）
k2pdfopt --dev kpw -m 0.3,0.5,0.3,0.5 input.pdf

# 用厘米
k2pdfopt --dev kpw -m 1cm input.pdf
```

### 场景 10：预览效果再决定要不要转

```sh
# --dry-run 显示转换计划但不实际处理
k2pdfopt --dev kpw --dry-run input.pdf
```

---

## 所有常用参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--dev <PROFILE>` | `kv` | 设备 profile（见下方设备列表） |
| `-o, --output <FMT>` | `%s_k2opt.pdf` | 输出文件名格式，`%s` = 原文件名 |
| `-p, --pages <RANGE>` | 全部 | 处理的页码（如 `1-10`, `1,3,5`, `even`, `odd`） |
| `--px <RANGE>` | 无 | 排除的页码（同上格式） |
| `-m, --margins <M>` | 0 | 源 PDF 裁剪边距，逗号分隔 L,T,R,B 或单值 |
| `--om <M>` | 设备决定 | 输出 PDF 边距 |
| `-c, --cover` | 关 | 在输出里包含封面页 |
| `-t, --trim` / `--no-t` | 开 | 自动裁剪源页边距 |
| `--fc` / `--no-fc` | 开 | 按屏幕宽度拟合列宽 |
| `--wrap` / `--no-wrap` | 关 | 文本换行 |
| `--ls` / `--no-ls` | 关 | 横屏（landscape）模式 |
| `-j, --justify <M>` | -1 | 0=左对齐，1=居中；`+`=两端对齐，`-`=不两端对齐（如 `1+`） |
| `--dpi <N>` | 设备决定 | 同时设置输入和输出 DPI |
| `--odpi <N>` | 设备决定 | 只设置输出 DPI |
| `-w, --width <W>` | 设备决定 | 输出宽度（支持单位 `in` / `cm` / `px`） |
| `--height <H>` | 设备决定 | 输出高度 |
| `--c` / `--no-c` | 关（灰度） | 彩色输出 |
| `--reflow <MODE>` | `auto` | 文本重排：`off` / `auto` / `force` |
| `--ocr <LANG>` | 关 | OCR 语言，详见下方 OCR 章节 |
| `--ocr-mode <MODE>` | `fallback` | OCR 缺语言策略，详见下方 OCR 章节 |
| `-x, --exit` | 关 | 转完直接退出（批处理用） |
| `-y, --yes` | 关 | 所有提示都自动选 yes |
| `-v, --verbose` | 关 | 啰嗦输出，可叠加：`-v` / `-vv` / `-vvv` |
| `--ui-` | 关 | 强制非交互（批处理用） |

查看所有 22 个 M1 参数 + 帮助说明：

```sh
k2pdfopt --help
```

查看与 C v2.55 的兼容性差异：

```sh
k2pdfopt --compat-report
```

输出当前参数等价的命令行（方便记下来下次用）：

```sh
k2pdfopt --dev kpw --ocr eng --reflow auto --echo-cmd
```

---

## 支持的设备 profile

用 `--dev <alias>` 选择，30 个内置 profile：

| 别名 | 设备 | 屏幕（像素） | DPI |
|------|------|--------------|-----|
| `k2` | Kindle 2 | 560×735 | 167 |
| `dx` | Kindle DX | 824×1000 | 167 |
| `kpw` | Kindle Paperwhite 1/2 | 658×889 | 212 |
| `kp2` | Kindle Paperwhite 2 | 758×1024 | 212 |
| `kp3` | Kindle Paperwhite 3 | 1072×1448 | 300 |
| `kv` | Kindle Voyage / Paperwhite 3+ | 1016×1364 | 300（**默认**） |
| `ko2` | Kindle Oasis 2/3 | 1200×1583 | 300 |
| `kbm` | Kindle Basic（10 代） | 600×800 | 167 |
| `kba` | Kindle Basic（11 代） | 1072×1448 | 300 |
| `kbhd` | Kindle Basic HD | 1264×1680 | 300 |
| `kbh2o` | Kindle Basic HD 2 | 1264×1680 | 300 |
| `kao` | Kindle Scribe | 1860×2480 | 300 |
| `koc` | Kobo Clara HD | 1072×1448 | 300 |
| `kof` | Kobo Forma | 1440×1920 | 300 |
| `kol` | Kobo Libra | 1264×1680 | 300 |
| `kbt` | Kindle Touch | 600×800 | 167 |
| `kbg` | Kindle Glow | 758×1024 | 212 |
| `kghd` | Kindle Glow HD | 1072×1448 | 300 |
| `pb2` | PocketBook | 600×800 | 167 |
| `nookst` | Nook Simple Touch | 600×800 | 167 |
| `nex7` | Nexus 7 | 800×1280 | 216 |
| ... | ... | ... | ... |

完整 30 个 profile 列表：

```sh
k2pdfopt --list-devices
```

部分匹配也可以：`--dev paperwhite` 会匹配 `kpw`（只要不重名）。

---

## OCR 完整指南

### 启用 OCR

加 `--ocr <语言代码>` 即可。语言代码是 Tesseract 的标准 3 字母代码：

| 语言 | 代码 |
|------|------|
| 英语 | `eng` |
| 中文简体 | `chi_sim` |
| 中文繁体 | `chi_tra` |
| 日语 | `jpn` |
| 韩语 | `kor` |
| 法语 | `fra` |
| 德语 | `deu` |
| 俄语 | `rus` |

多语言用 `+` 连接：`--ocr "chi_sim+eng"`。

### 缺语言时的策略：`--ocr-mode`

如果你的 tesseract 没装某个语言包，可以选三种处理方式：

| 模式 | 行为 | 适用场景 |
|------|------|----------|
| `fallback`（默认） | 缺的语言自动退回 `eng`，给警告 | 不确定装了哪些语言时 |
| `partial` | 用已装的语言，跳过缺的，给警告 | 多语言但允许部分缺失 |
| `strict` | 缺一个就报错退出（退出码 1） | 自动化脚本，要确保语言齐全 |

示例：

```sh
# 严格模式：缺 jpn 直接报错
k2pdfopt --dev kpw --ocr "jpn+kor" --ocr-mode strict input.pdf
```

### 高级 OCR 选项

```sh
# 调整 OCR 输出可见性（bit mask）
# 1 = 显示源 bitmap（默认）
# 2 = 显示 OCR 文本层（用于选中复制）
# 4 = 显示识别框
# 8 = 用空格分词
# 16 = 优化空格
# 常用：3 = 源 + 文本层（推荐），7 = 源 + 文本 + 框（调试用）
k2pdfopt --dev kpw --ocr eng --ocr-visibility-flags 3 input.pdf

# 丢弃低置信度的词（0.0 = 不过滤，0.5 = 丢弃置信度低于 50% 的词）
k2pdfopt --dev kpw --ocr eng --ocr-min-confidence 0.5 input.pdf
```

### OCR 中断

OCR 处理大文档时间长。**按 Ctrl-C 即可中断**，会立即终止当前的 tesseract 子进程，1 秒内退出。

---

## 常见问题

### Q1：报错 `mutool not found` 怎么办？

A：mupdf-tools 没装或不在 PATH 里。按上方 [安装第 2 步](#第-2-步装运行时依赖) 安装。装完后在新终端验证 `mutool -v`。

### Q2：用 `--ocr chi_sim` 报错 "language not found"？

A：tesseract 没装中文语言包。

- Windows: `scoop install tesseract-languages`
- macOS: `brew install tesseract-lang`
- Linux: `sudo apt install tesseract-ocr-chi-sim`

装完用 `tesseract --list-langs` 验证。

### Q3：输出 PDF 太大 / 太模糊？

A：调整 DPI：

```sh
# 提高 DPI 让字更清晰（但文件更大）
k2pdfopt --dev kpw --dpi 300 input.pdf

# 降低 DPI 让文件更小
k2pdfopt --dev kpw --dpi 150 input.pdf
```

### Q4：双列 PDF 输出页码异常 / 顺序错乱？

A：试试强制 reflow：

```sh
k2pdfopt --dev kpw --reflow force input.pdf
```

如果还有问题，可能是源 PDF 的列检测失败，尝试关掉 reflow：

```sh
k2pdfopt --dev kpw --reflow off input.pdf
```

### Q5：能处理加密 PDF 吗？

A：能读，但**不能写入**加密 PDF。如果源是加密的，输出会是未加密版本（前提是你有权解密）。

### Q6：进度条不显示？

A：可能是终端不支持。加 `-vv` 切换成纯文本进度输出：

```sh
k2pdfopt --dev kpw -vv input.pdf
```

### Q7：能在 PowerShell 5.1（老 Windows）上跑吗？

A：目前没专门测试，建议用 PowerShell 7+ 或 cmd.exe。Git Bash / WSL 也可以。

### Q8：怎么取消正在转的任务？

A：按 Ctrl-C。会在下一个安全检查点退出（通常 1 秒内）。OCR 模式下子进程也会被立即 kill。

---

## Shell 自动补全

`completions/` 目录下有 5 种 shell 的自动补全脚本（bash / zsh / fish / powershell / elvish）。

### Bash

```sh
mkdir -p ~/.local/share/bash-completion/completions
cp completions/bash/k2pdfopt.bash ~/.local/share/bash-completion/completions/k2pdfopt
# 重开终端
```

### Zsh

```sh
mkdir -p ~/.zfunc
cp completions/zsh/_k2pdfopt ~/.zfunc/
# 在 ~/.zshrc 加：
# fpath=(~/.zfunc $fpath)
# autoload -Uz compinit && compinit
```

### Fish

```sh
mkdir -p ~/.config/fish/completions
cp completions/fish/k2pdfopt.fish ~/.config/fish/completions/
```

### PowerShell

```powershell
# 当前会话
. completions/powershell/k2pdfopt.ps1

# 永久启用
Add-Content -Path $PROFILE -Value '. "<repo path>/completions/powershell/k2pdfopt.ps1"'
```

### Elvish

```sh
mkdir -p ~/.config/elvish/lib
cp completions/elvish/k2pdfopt.elv ~/.config/elvish/lib/
# 在 ~/.config/elvish/rc.elv 加：use k2pdfopt
```

---

## 退出码

写脚本判断结果用：

| 退出码 | 含义 |
|--------|------|
| 0 | 成功 |
| 1 | 用户错误（参数不对、`--ocr-mode strict` 缺语言等） |
| 2 | 处理错误（PDF 损坏、磁盘满等） |
| 10 | 内部错误（bug，请反馈） |
| 130 | 被 Ctrl-C 中断 |

---

## 环境变量

可以用 `K2PDFOPT` 环境变量预设默认参数：

```sh
# Linux / macOS
export K2PDFOPT="--dev kpw --ocr eng -x"
k2pdfopt input.pdf    # 等价于 k2pdfopt --dev kpw --ocr eng -x input.pdf

# Windows PowerShell
$env:K2PDFOPT = "--dev kpw --ocr eng -x"
```

命令行参数优先级 > 环境变量 > 默认值。

---

## License

 [AGPL-3.0-or-later](LICENSE)


---

## Credits

- 上游 C 版作者 [Willus Hull](https://www.willus.com/k2pdfopt/) — 本 Rust 重写以 v2.55 为参照
- 核心依赖：
  - [mupdf-tools](https://mupdf.com) — PDF 渲染
  - [Tesseract OCR](https://github.com/tesseract-ocr/tesseract) — OCR 引擎
  - [lopdf](https://github.com/J-F-Liu/lopdf) — PDF 写入
  - [clap](https://docs.rs/clap) — CLI 参数解析
  - [indicatif](https://docs.rs/indicatif) — 进度条
  - [tracing](https://docs.rs/tracing) — 日志

---

## 反馈 / 贡献

发现 bug 或想加功能？欢迎提 Issue 或 PR。

- 贡献代码请先跑 `cargo fmt --check && cargo clippy -- -D warnings && cargo test --workspace` 确保通过
- 影响渲染输出的 PR 请额外跑 `cargo run --release --bin run_regression -- --all` 确保 12 个 fixture 全过
