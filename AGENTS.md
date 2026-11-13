# AGENTS.md

> 本文件是所有 AI 编码助手(Claude Code、Cursor、Copilot 等)在本仓库工作的**唯一入口**。
> 开始任何任务前先读完本文件,再按任务类型读 `docs/ai/` 下对应文档。

## 1. 项目是什么

基于 Rust + GPUI 的跨平台终端模拟器。第一阶段目标:复刻 KDE Konsole 的界面与行为。
Windows 与 Linux **同等优先**,任何功能都必须在两个平台上编译并工作(或明确标注平台差异)。

最重要的单项功能:**选中文本自动进入选择缓冲区,鼠标中键粘贴**(规格见 `BEHAVIOR_SPEC.md`)。

## 2. 文档索引

| 什么时候读 | 文档 |
|---|---|
| 每次任务开始 | 本文件、`docs/ai/WORKFLOW.md` |
| 改动模块边界、线程、数据流 | `docs/ai/ARCHITECTURE.md` |
| 写任何 Rust 代码 | `docs/ai/CODING_STANDARDS.md` |
| 实现或修改用户可见行为 | `docs/ai/BEHAVIOR_SPEC.md` |
| 写测试、验收 | `docs/ai/TESTING.md` |
| 参考其他项目、引入依赖或代码 | `docs/ai/LICENSING.md` |

文档之间冲突时的优先级:**用户当前指令 > 本文件硬性规则 > BEHAVIOR_SPEC > ARCHITECTURE > CODING_STANDARDS > 其他**。发现文档互相矛盾或已过时,停下来报告,不要自行选一个。

## 3. 硬性规则(不可违反,没有例外)

1. **不凭记忆写第三方 API。** GPUI 与 `alacritty_terminal` 的 API 变化频繁。使用前必须在锁定版本的源码中确认(`~/.cargo/registry/src/` 或 git 依赖目录),确认不了就明说"未验证",不得编造方法名、参数或类型。
2. **不复制其他项目的代码。** Konsole(GPL)、Windows Terminal(MIT)、Zed(各 crate 许可证不同)只能作为行为和设计参考。详见 `LICENSING.md`。
3. **不擅自新增依赖、不升级依赖版本、不改 `rust-toolchain`。** 需要时在汇报里提出理由,等用户批准。
4. **不写 `unsafe`**,除非任务明确要求并附带安全性说明(`// SAFETY:` 注释)。
5. **持有 `Term` 锁期间,禁止:** 做 I/O、调用 GPUI 的 `cx`/`window`、`await`、克隆大块数据。拿到所需数据后立即释放锁。
6. **平台对等。** `#[cfg(target_os = ...)]` 只允许出现在 `platform` 模块内,业务代码通过 trait 调用。新代码不得让任一平台编译失败。
7. **中键粘贴语义不得退化。** 任何触及选区、剪贴板、鼠标事件的改动,必须跑 `BEHAVIOR_SPEC.md` 中 `SEL-*` / `PASTE-*` 相关测试。
8. **粘贴安全过滤不得削弱。** 不得移除对 ESC 及危险控制字符的处理(`PASTE-004`、`PASTE-005`)。
9. **不为了让检查通过而改检查。** 不得删除或弱化测试、不得用 `#[allow(...)]` 掩盖 clippy 警告、不得 `#[ignore]` 失败的测试。测试错了就说明并修正测试,且在汇报中单独列出。
10. **只改任务范围内的文件。** 不顺手重构、不顺手格式化无关文件、不重命名公共接口(除非任务要求)。
11. **不执行破坏性操作:** 不 `git push --force`、不 `git reset --hard`、不删除未提交的工作、不修改 `.git` 配置、不提交密钥或本机路径。
12. **不确定就问。** 需求有歧义、规格缺失、两种实现各有代价时,先提问或列出方案,不要自行猜一个并默默实现。

## 4. 工作方式(摘要,完整流程见 WORKFLOW.md)

1. 读任务 → 读相关文档 → 读相关现有代码。
2. 用不超过 10 行写出计划与影响范围;任务较大时等用户确认。
3. 小步实现,每步都能编译、测试。
4. 跑完验证命令,按汇报模板报告结果,**如实说明没做到的部分和没验证的部分**。

## 5. 常用命令

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace                 # 当前平台
# 交叉检查(无需运行,只检查能否编译)
cargo check --workspace --target x86_64-pc-windows-msvc
cargo check --workspace --target x86_64-unknown-linux-gnu
```

> 如果某条命令在当前环境跑不了(缺系统库、缺目标平台工具链),在汇报中写明"未运行"及原因,不得声称已通过。

## 6. 沟通约定

- 用中文回复;代码标识符用英文;代码注释用中文,解释"为什么"而不是"做了什么"。
- 汇报只写事实:改了什么、验证了什么、没验证什么、有什么风险。不写夸张的总结。
- 引用代码位置用 `路径:行号`。
- 不确定的结论必须带"未验证"或"推测"标记。
