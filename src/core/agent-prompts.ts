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

export type AgentMode = 'off' | 'normal' | 'auto' | 'plan';

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
        `- delegate_task(profile, task, context_files?) — 创建子智能体执行独立子任务`,
        ``,
        `子智能体使用指南:`,
        `- 面对复杂任务时，先分析是否可以拆分为独立子任务`,
        `- 同一轮中调用多个 delegate_task 时，子智能体会**并行执行**，同时工作`,
        `- 识别可并行的独立子任务后，一次性创建多个子智能体以节省时间`,
        `- 示例：搜索代码库 + 分析架构 → 同时创建多个 explorer 子智能体分别搜索不同模块`,
        `- explorer（代码探索者）：只读搜索和分析代码，适合探索代码库、查找相关文件、分析架构`,
        `- coder（代码实现者）：编写和修改代码，适合具体功能实现。禁止执行 shell 命令`,
        `- general（通用子智能体）：全能力，适合需要混合操作的子任务`,
        `- 子智能体独立执行，完成后返回工作总结供你参考`,
        `- 注意：并行子智能体之间无法通信，确保每个子任务是真正独立的`,
        `- 不要创建嵌套子智能体（子智能体不能再创建子智能体）`,
        ``,
    ];

    // 模式特定的行为指令
    switch (mode) {
        case 'normal':
            lines.push(
                `当前是普通模式：文件读取/搜索可以自由执行，写入/删除文件前需要用户确认。`,
                `在调用 write_file、delete_file、make_directory 等修改性工具之前，`,
                `你必须先向用户说明要修改的内容并获得明确批准。`,
            );
            break;
        case 'auto':
            lines.push(
                `当前是自动模式：尽可能自主完成任务，仅在遇到无法处理的错误时才向用户求助。`,
                `你可以自由使用所有可用工具来达成目标。`,
            );
            break;
        case 'plan':
            lines.push(
                `当前是 Plan 模式：请先列出任务计划和涉及的文件（不要调用工具），`,
                `等用户确认后再执行。你的职责是分析需求、制定方案，而不是直接修改文件。`,
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

    const lines: string[] = [
        `[Agent Mode] 工作目录: ${project.path}`,
        `当前模式: ${mode === 'auto' ? '自动' : mode === 'normal' ? '普通' : 'Plan'}`,
        mode === 'plan' ? `Plan 模式：请继续制定计划，不要调用工具。` :
        mode === 'normal' ? `普通模式：修改文件前需要用户确认。` :
        `自动模式：自主完成任务。`,
        `所有文件路径相对于项目根目录，不可越界。`,
    ];

    return lines.join('\n');
}
