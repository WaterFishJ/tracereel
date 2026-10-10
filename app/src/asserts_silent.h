// 让 MSVC 的断言框**不弹出来，同时让 Qt 继续跑**（等价于人工点「忽略」）。
//
// 背景：gui.qml 实例化时会触发 Qt 的 `Q_ASSERT("bit < 2 * props")`
// （qqmlengine.cpp:1441，QQmlData::growBits）。Qt 在 Windows 上把这个断言交给 CRT 的
// `_CrtDbgReport(_CRT_ASSERT, ...)`，它默认弹一个模态框：
//   - 点「忽略」→ 返回 0 → Qt 继续
//   - 点「中止」→ 返回 1 → Qt abort → 进程死
//   - 把报告模式改成 FILE / DEBUG → 没有框，但返回 1 → 进程直接 fail-fast（0xC0000409）
// 也就是说**只有「窗口模式 + 人点忽略」才等于返回 0**，没有弹框就没法继续 —— 除非自己接
// 报告钩子：`_CrtSetReportHook` 的钩子返回 TRUE 时，`_CrtDbgReport` 会把
// `*returnValue` 当作结果返回。于是钩子里把 `*returnValue = 0`，
// 就等于「无人值守地一直点忽略」：不弹框、进程照常往下走。
//
// ⚠️ 只在「在意无人值守」的场景装钩子。GUI 目前没装（保持原来的行为：弹框→人工点忽略），
// 因为装了以后断言就彻底静默了；无头工具（qmlprobe / guirender / encgui …）都装。
#pragma once

#include <windows.h>
#include <crtdbg.h>
#include <cstdio>
#include <cstdint>

// 报告钩子：消息打到 stderr，返回值置 0 = 让 CRT 当成「忽略」。
static int __cdecl ov_report_hook(int reportType, char *message, int *returnValue)
{
    (void)reportType;
    if (message)
        std::fprintf(stderr, "%s\n", message);
    std::fflush(stderr);
    if (returnValue)
        *returnValue = 0;  // 0 = Ignore（1 = Abort）；Qt 收到 0 就继续
    return TRUE;           // 已处理：不要再弹框
}

static inline void ov_asserts_silent_continue()
{
    _set_error_mode(_OUT_TO_STDERR);
    _set_abort_behavior(0, _WRITE_ABORT_MSG | _CALL_REPORTFAULT);

    HMODULE crt = ::LoadLibraryA("ucrtbased.dll");
    if (!crt)
        crt = ::GetModuleHandleA("ucrtbased.dll");
    if (crt) {
        typedef int(__cdecl * set_hook_fn)(int(__cdecl *)(int, char *, int *));
        set_hook_fn setHook = (set_hook_fn)::GetProcAddress(crt, "_CrtSetReportHook");
        if (setHook)
            setHook(&ov_report_hook);
    }
}
