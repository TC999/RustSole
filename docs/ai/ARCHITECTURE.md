# ARCHITECTURE:架构与不变量

> 本文件定义模块边界。修改这里的任何"不变量"都属于 L 级任务,必须先获得用户确认。

## 1. 分层

```
GPUI View / Element      ← 只负责输入采集与渲染
        │
Session(独立于 View)    ← 一个 PTY + 一个 Term + 事件流
        │
alacritty_terminal       ← VT 解析、Grid、选区数据结构
        │
PTY(ConPTY / openpty)
```

**不自己实现 VT 解析器。** 转义序列、备用屏、滚动区域、宽字符都交给 `alacritty_terminal`。

## 2. 建议的 crate 划分

```
crates/
  term_core/       Session、Term 封装、事件、输入编码(键盘→字节)、粘贴处理
  term_platform/   选择缓冲区、剪贴板、PTY 差异、Profile 自动发现(所有 cfg 集中在这里)
  term_config/     配置结构、解析、分层合并、校验
  term_ui/         GPUI 视图、Element、标签栏、分屏、菜单、滚动条
  app/             入口、Action 注册、窗口管理
```

依赖方向只能是:`app → term_ui → term_core → term_platform`,`term_config` 被上层读取。
**`term_core` 不得依赖 `gpui`。** 这样核心逻辑可以脱离图形环境做单元测试。

## 3. 不变量

1. **Session 独立于 View。** Session 持有 PTY 与 Term 的所有权;View 只持有引用并负责显示。这是标签拖拽、跨窗口移动、分屏重排的前提。
2. **Term 锁的临界区要短。** 渲染时在锁内只提取"可绘制快照"(单元格、光标、选区范围),然后立即释放,绘制在锁外进行。
3. **PTY 读写永远不在 GPUI 主线程。** `alacritty_terminal` 的 `EventLoop` 在独立线程运行,通过 channel 向主线程投递事件。
4. **Wakeup 必须节流。** 大量输出时合并重绘请求(目标约 8 ms 一次),不得每个 Wakeup 都 `cx.notify()`。
5. **所有用户操作走 Action。** 快捷键、菜单、右键菜单、(将来的)命令面板都触发同一批 Action,业务逻辑只写在 Action 处理函数里,不得写在事件回调里重复一份。
6. **平台代码集中。** `cfg(target_os)` 只在 `term_platform` 内。`term_platform` 对外只暴露 trait 与统一类型。
7. **选择缓冲区与系统剪贴板是两个独立通道。** 任何代码不得把它们合并或互相覆盖(见 `BEHAVIOR_SPEC.md`)。
8. **渲染用自定义 `Element`,不用 `div` 逐格拼接。** 相邻同样式单元格合并为一个文本 run。
9. **配置是只读快照。** 视图读取配置的 `Arc` 快照,热重载时整体替换,不在渲染中途修改。

## 4. 线程模型

| 线程 | 职责 | 禁止 |
|---|---|---|
| GPUI 主线程 | 输入、布局、绘制、Action 处理 | 阻塞 I/O、长时间持锁 |
| PTY 事件循环线程 | 读写 PTY、喂给 Term | 调用 GPUI API |
| 后台任务(`cx.background_spawn`) | 文件读写、Profile 发现、搜索 | 直接修改 UI 状态 |

跨线程通信:channel 或 `cx.spawn` 回到主线程后再 `update` 实体。

## 5. 核心数据流

**输出:** 子进程 → PTY → EventLoop 线程 → `Term`(加锁更新)→ `Wakeup` 事件 → 主线程节流 → `cx.notify()` → Element 重新取快照并绘制。

**输入:** GPUI 键盘事件 → 键位编码 → `Session::write(bytes)` → EventLoop 线程 → PTY。

**选中:** 鼠标事件 → 更新 `Term.selection` → 左键抬起 → 取选中文本 → `SelectionBuffer::set`。

**中键粘贴:** 鼠标事件 → `SelectionBuffer::get` → `paste_text`(换行规范化、过滤、bracketed paste)→ `Session::write`。

## 6. 允许与禁止

允许:新增 trait 隔离平台差异;新增内部模块;为性能增加缓存(需有度量依据)。

禁止:在 `term_core` 引入 `gpui`;在多个地方各写一份粘贴处理;让 View 直接操作 PTY;用全局可变静态变量保存终端状态(`Global` 仅限不可变配置和选择缓冲区)。
