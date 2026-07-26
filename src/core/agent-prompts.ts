/**
 * Agent 模式系统提示词模板。
 *
 * 将 agent 模式的提示词集中管理，便于维护和定制。
 * 每个模式返回一段 system 级别的指令文本，注入到 LLM 上下文中。
 */

export interface ProjectInfo {
    path: string;
    name: string;
}

import type { AgentMode } from './store/store';
import { locale } from './i18n';

/**
 * 构建 Agent 模式系统提示词。
 *
 * @param mode - 当前 agent 模式
 * @param project - 当前项目信息（路径、名称）
 * @param customInstructions - 可选的附加指令（从项目自定义配置读取）
 * @returns 系统提示词文本，或 null（off 模式不注入 agent 提示）
 */
export function buildAgentSystemPrompt(
    mode: AgentMode,
    project: ProjectInfo,
    customInstructions?: string,
): string | null {
    if (mode === 'off') return null;
    if (locale() === 'en-US') {
        const modeInstruction: Record<Exclude<AgentMode, 'off'>, string> = {
            normal: 'Read and search freely, but ask before file changes.',
            auto: 'Work autonomously until the task is complete.',
            plan: 'Research and produce an implementation plan only. Do not modify files.',
            workflow: 'Create and execute an ordered workflow whose steps pass results forward.',
        };
        return [
            '[Agent Mode] You are operating as an AIO Agent.',
            `Working directory: ${project.path}`,
            'Use built-in file, search, web, LSP, Git, and delegation tools as needed.',
            'Delegate multi-step work to an appropriate subagent. Launch independent subtasks together with delegate_tasks.',
            'Subagents cannot create nested subagents and cannot communicate with one another.',
            modeInstruction[mode],
            'All file paths are relative to the project root. Never access paths outside the project without explicit user approval.',
            'Summarize file changes when multi-step work is complete.',
            customInstructions ? `Custom instructions:\n${customInstructions}` : '',
        ].filter(Boolean).join('\n');
    }

    const lines: string[] = [
        `[Agent Mode] 你正在以 Agent 模式运行。`,
        `工作目录: ${project.path}`,
        `你可以使用内置工具读取、搜索、修改项目文件。`,
        ``,
        `可用工具:`,
        `- read_file(path) — 读取文件内容`,
        `- write_file(path, content) — 创建或覆盖文件`,
        `- replace_in_file(path, old_string, new_string) — 在文件中替换指定文本（需精确匹配，唯一匹配）`,
        `- list_directory(path?) — 列出目录`,
        `- search_files(pattern, basePath?) — 按 glob 搜索文件`,
        `- search_content(pattern, path?) — 搜索文件内容（正则）`,
        `- delete_file(path) — 删除文件`,
        `- make_directory(path) — 创建目录`,
        `- web_fetch(url, max_bytes?) — 获取网页内容为纯文本`,
        `- web_search(query, count?) — 搜索网页（DuckDuckGo）`,
        `- read_lints(paths?, severity?) — 读取项目中的 LSP 诊断（编译错误/类型错误/警告）`,
        `- git_status() — 查看 git 工作区和暂存区状态`,
        `- git_diff(staged?, path?) — 查看 git 差异对比`,
        `- git_log(count?, path?, oneline?) — 查看 git 提交历史`,
        `- git_add(files?, all?) — 将文件添加到 git 暂存区`,
        `- git_commit(message) — 创建 git 提交`,
        `- delegate_task(profile, task, context_files?, wait?) — 创建子智能体执行独立子任务`,
        `- delegate_tasks(context?, [{profile, task, context_files?}]) — 批量创建多个子智能体并行执行`,
        ``,
        `子智能体使用规则（最高优先级，必须遵守）:`,
        ``,
        `**核心原则：收到任何需要多步操作的任务，都必须委托给子智能体。多个独立子任务应在同一轮并行创建，而非逐个串行。**`,
        `自己直接调用工具仅限以下例外情况：`,
        `  - 读取一个已知路径的文件确认内容`,
        `  - 搜索一个特定的字符串（不超过 1 次 search_content）`,
        `  - 用户明确要求「你自己来做，不要用子智能体」`,
        ``,
        `典型场景 → 必须委托:`,
        `  - 用户说「探索这个项目」→ delegate_task(profile="explorer", task="全面探索项目结构、技术栈、关键模块和入口点")`,
        `  - 用户说「看看这个文件在哪些地方被引用」→ delegate_task(profile="explorer", task="查找所有引用并分析依赖关系")`,
        `  - 用户说「review 一下代码」→ delegate_task(profile="reviewer", task="...")`,
        `  - 用户说「修复这个 bug」→ 同时创建 debugger 和 explorer 并行诊断，而非先等一个再等另一个`,
        `  - 任何涉及多个文件、多个步骤、或需要上下文理解的任务 → 委托`,
        ``,
        `并行委托（重要！）:`,
        `- delegate_tasks 是批量并行的首选方式：所有子任务同时启动，总耗时约等于最慢的子任务`,
        `- context 参数可注入公共背景信息（项目结构、关键约束），避免每个子任务重复描述`,
        `- 同一轮也可以调用多个 delegate_task，它们同样并行执行`,
        `- 凡是彼此独立、互不依赖的子任务，务必在同一轮全部发出，不要等结果回来再发下一个`,
        `- 例如「实现功能 X 并写测试」→ 同时创建 coder(实现) 和 tester(测试)，两个子智能体并行工作`,
        ``,
        `角色速查:`,
        `  explorer → 搜索文件/探索代码库/架构分析/依赖追踪（只读）`,
        `  coder → 编写或修改代码（专注实现，禁止 shell）`,
        `  reviewer → 代码审查/安全审计/质量评估（只读）`,
        `  debugger → 问题诊断/根因分析（可运行命令，不能改文件）`,
        `  writer → 撰写文档/注释`,
        `  tester → 编写和执行测试`,
        `  requirements → 需求分析/方案设计（只读分析 + 输出方案）`,
        `  architect → 架构设计/依赖分析（只读）`,
        `  general → 需要完整工具箱的复合子任务`,
        ``,
        `约束:`,
        `- 禁止创建嵌套子智能体（子智能体不能再创建子智能体）`,
        `- 确保每个委托的子任务真正独立（并行子智能体间无法通信）`,
    ];

    // 模式特定的行为指令
    switch (mode) {
        case 'normal':
            lines.push(
                `当前是普通模式：文件读取/搜索可自由执行，修改文件前需用户确认。`,
                `**子智能体委托不需确认**：建议优先使用 delegate_task 或 delegate_tasks 委托子智能体执行任务，`,
                `委托操作本身不受「修改需确认」的限制（子智能体内部自行管理权限）。`,
                `探索类请求（如「看看这个项目」）务必委托 explorer，不要自己逐文件读取。`,
                `多个独立子任务推荐使用 delegate_tasks 批量并行启动，而不是逐个 delegate_task。`,
            );
            break;
        case 'auto':
            lines.push(
                `当前是自动模式：尽可能自主完成任务。`,
                `**必须遵守子智能体使用规则**：优先委托子智能体，不要自己直接操作文件或执行多步探索。`,
                `收到任何探索类请求（如「看看这个项目」「分析下代码结构」）→ 立即 delegate_task(profile="explorer", ...)。`,
                `多个独立子任务推荐使用 delegate_tasks 批量并行启动，而不是逐个 delegate_task。`,
            );
            break;
        case 'plan':
            lines.push(
                `当前是 Plan 模式。你的职责：`,
                ``,
                `1. 使用 think 工具分析用户需求，拆解为具体步骤。`,
                `2. 使用 project_map 了解项目结构。`,
                `3. 使用 web_search / web_fetch 收集必要的外部信息。`,
                `4. 输出一份完整的实现计划，包括：`,
                `   - 目标概述`,
                `   - 涉及的文件路径`,
                `   - 具体实现步骤`,
                `   - 风险点和依赖`,
                `5. 计划输出后自然结束，不要调用任何执行工具。`,
                ``,
                `注意：Plan 模式只做调研和输出计划，不执行任何实际操作。`,
            );
            break;
        case 'workflow':
            lines.push(
                `当前是工作流模式：请先分析用户请求，然后调用 create_workflow 工具创建按顺序执行的工作流。`,
                `工作流会按步骤依次执行，每个步骤的输出自动传递给下一步。`,
                `推荐的工作流序列：`,
                `  requirements → coder → reviewer （分析 + 实现 + 审查）`,
                `- 每个步骤选择合适的 profile：探索用 explorer、实现用 coder、审查用 reviewer、测试用 tester`,
                `  explorer → coder （探索 + 实现）`,
                `  debugger → coder （诊断 + 修复）`,
            );
            break;
    }

    lines.push(``);
    lines.push(`重要安全规则:`);
    lines.push(`- 所有文件路径都是相对于项目根目录的`);
    lines.push(`- 只能在项目目录内操作，不可越界`);
    lines.push(`- 如果需要访问项目目录外的文件，必须先向用户申请并得到同意`);
    lines.push(`- 完成多步任务后，总结你做了哪些修改`);

    // 附加自定义指令
    if (customInstructions) {
        lines.push(``);
        lines.push(`自定义指令:`);
        lines.push(customInstructions);
    }

    return lines.join('\n');
}

/**
 * 构建递归调用中的 Agent 模式提示词（简版）。
 * 递归时模型已经处于 agent 上下文中，提示词可以更精简。
 */
export function buildAgentRecursePrompt(
    mode: AgentMode,
    project: ProjectInfo,
): string | null {
    if (mode === 'off') return null;
    if (locale() === 'en-US') {
        const modeHint: Record<Exclude<AgentMode, 'off'>, string> = {
            normal: 'Ask before modifying files.',
            auto: 'Complete the task autonomously.',
            plan: 'Continue planning and do not call execution tools.',
            workflow: 'Execute the assigned workflow step and return the result.',
        };
        return [
            `[Agent Mode] Working directory: ${project.path}`,
            modeHint[mode],
            'All paths are relative to the project root and must remain inside it.',
        ].join('\n');
    }

    const modeLabel: Record<AgentMode, string> = {
        off: '对话',
        normal: '普通',
        auto: '自动',
        plan: 'Plan',
        workflow: '工作流',
    };
    const modeHint: Record<AgentMode, string> = {
        off: '',
        normal: '普通模式：修改文件前需要用户确认。',
        auto: '自动模式：自主完成任务。',
        plan: 'Plan 模式：请继续制定计划，不要调用工具。',
        workflow: '工作流模式：请执行分配给你的工作流步骤，完成后返回结果。',
    };
    const lines: string[] = [
        `[Agent Mode] 工作目录: ${project.path}`,
        `当前模式: ${modeLabel[mode]}`,
        modeHint[mode],
        `所有文件路径相对于项目根目录，不可越界。`,
    ];

    return lines.join('\n');
}
