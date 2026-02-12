/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * 用户登录/注册模态框组件，支持邮箱密码登录、新用户注册、成功状态动画反馈。
 * 提供表单验证、错误提示、加载状态、进入/退出动画等完整交互体验。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  外部数据流入 (Props)                                                    │
 * │  ├── show: boolean ← 父组件控制模态框显示/隐藏                           │
 * │  ├── onClose: () => void ← 关闭回调                                      │
 * │  └── onSuccess: (userData: any) => void ← 登录成功回调，传出用户信息     │
 * │                                                                          │
 * │  用户交互输出                                                            │
 * │  ├── invoke('register_to_backend') → Tauri 后端命令：注册新用户          │
 * │  ├── invoke('login_to_backend') → Tauri 后端命令：用户登录验证           │
 * │  └── onSuccess/onClose → 通过 Props 回调通知父组件                       │
 * │                                                                          │
 * │  本地状态                                                                │
 * │  ├── isRegister: 当前模式（登录/注册）                                   │
 * │  ├── email/username/password/confirmPassword: 表单输入                   │
 * │  ├── loading: 提交中状态（禁用按钮）                                     │
 * │  ├── error: 错误提示信息                                                 │
 * │  ├── isSuccess: 操作成功状态（显示成功动画）                             │
 * │  ├── isLeaving: 成功动画离开状态                                         │
 * │  └── isExiting: 模态框退出动画状态                                       │
 * │                                                                          │
 * │  本地文件                                                                │
 * │  └── 导入 LoginModal.css 样式文件                                        │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * LoginModal (本组件)
 * ├── 遮罩层 (点击关闭，带动画)
 * ├── 模态框容器 (带动画)
 * │   ├── 成功状态覆盖层 (条件渲染，SVG 动画)
 * │   ├── 头部 (标题 + 关闭按钮)
 * │   ├── 表单区域
 * │   │   ├── 邮箱输入
 * │   │   ├── 密码输入
 * │   │   ├── 确认密码 (注册模式)
 * │   │   ├── 错误提示
 * │   │   └── 提交按钮
 * │   └── 底部切换链接
 * ============================================================================
 */

// SolidJS 核心 API
import { createSignal, Component, Show } from 'solid-js';
// Tauri 核心 API：调用 Rust 后端命令
import { invoke } from '@tauri-apps/api/core';
// 本地样式文件
import './LoginModal.css';

/**
 * 组件 Props 接口定义
 */
interface LoginModalProps {
    /** 控制模态框显示/隐藏 */
    show: boolean;
    /** 关闭模态框回调 */
    onClose: () => void;
    /**
     * 登录成功回调
     * @param userData - 后端返回的用户信息对象
     */
    onSuccess: (userData: any) => void;
}

/**
 * 登录/注册模态框组件
 * 
 * @component
 * @description 提供完整的登录注册功能，包含表单验证、后端交互、成功动画。
 *              使用 Tauri invoke 与 Rust 后端通信。
 * 
 * @param {LoginModalProps} props - 组件属性
 * @returns {JSX.Element} 登录模态框 JSX 元素
 */
const LoginModal: Component<LoginModalProps> = (props) => {
    // ==================== 本地状态定义 ====================

    /** 当前模式：false=登录，true=注册 */
    const [isRegister, setIsRegister] = createSignal(false);
    /** 邮箱输入值 */
    const [email, setEmail] = createSignal('');
    /** 用户名（当前未使用，预留字段） */
    const [username, setUsername] = createSignal('');
    /** 密码输入值 */
    const [password, setPassword] = createSignal('');
    /** 提交加载状态：true 时禁用按钮显示"请稍候" */
    const [loading, setLoading] = createSignal(false);
    /** 错误提示信息，非空时显示错误样式 */
    const [error, setError] = createSignal('');
    /** 确认密码（仅注册模式使用） */
    const [confirmPassword, setConfirmPassword] = createSignal('');
    /** 操作成功状态：true 时显示成功动画覆盖层 */
    const [isSuccess, setIsSuccess] = createSignal(false);
    /** 成功动画离开状态：控制成功覆盖层的退出动画 */
    const [isLeaving, setIsLeaving] = createSignal(false);
    /** 模态框退出动画状态：控制整体关闭动画 */
    const [isExiting, setIsExiting] = createSignal(false);

    // ==================== 交互处理函数 ====================

    /**
     * 切换登录/注册模式
     * 
     * 切换时清理相关状态：
     * - 清空错误信息
     * - 清空密码字段（安全考虑）
     */
    const toggleMode = () => {
        setIsRegister(!isRegister());
        setError('');
        setPassword('');
        setConfirmPassword('');
    };

    /**
     * 处理关闭模态框（带动画）
     * 
     * 动画流程：
     * 1. 设置 isExiting=true 触发退出动画（CSS 类 overlay-out/animate-out）
     * 2. 等待 200ms（CSS 动画时长）
     * 3. 重置状态并调用父组件 onClose
     */
    const handleClose = () => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false);
            props.onClose();
        }, 200);
    };

    /**
     * 表单提交处理（登录或注册）
     * 
     * 数据流：
     * 1. 阻止默认表单提交行为
     * 2. 设置加载状态，清空旧错误
     * 3. 根据 isRegister 状态分支处理：
     *    - 注册：验证密码一致性 → 调用 register_to_backend → 显示成功 → 自动切换到登录
     *    - 登录：调用 login_to_backend → 显示成功 → 回调 onSuccess
     * 4. 捕获异常显示错误信息
     * 5. 最终关闭加载状态
     * 
     * @param {Event} e - 表单提交事件
     */
    const handleSubmit = async (e: Event) => {
        e.preventDefault();
        setLoading(true);
        setError('');

        try {
            if (isRegister()) {
                // ==================== 注册逻辑 ====================
                
                // 前端验证：密码一致性检查
                if (password() !== confirmPassword()) {
                    throw new Error("两次输入的密码不一致");
                }
                
                // 调用 Tauri 后端命令：register_to_backend
                await invoke('register_to_backend', {
                    email: email(),
                    password: password(),
                    confirmPassword: confirmPassword()
                });
                
                // 注册成功：显示成功动画
                setIsSuccess(true);
                
                // 延时处理：先显示成功状态 800ms，再播放离开动画
                setTimeout(() => {
                    setIsLeaving(true); // 触发离开动画
                    setTimeout(() => {
                        // 清理状态并切换到登录模式
                        setIsSuccess(false);
                        setIsLeaving(false);
                        setIsRegister(false);
                        setPassword('');
                        setConfirmPassword('');
                    }, 300); // 离开动画时长 300ms
                }, 800);

            } else {
                // ==================== 登录逻辑 ====================
                
                // 调用 Tauri 后端命令：login_to_backend
                // 注意：后端使用 username 字段接收邮箱
                const result: any = await invoke('login_to_backend', {
                    username: email(),
                    password: password()
                });
                
                // 登录成功：显示欢迎动画
                setIsSuccess(true);
                
                // 延时处理：先显示成功状态 600ms，再播放离开动画
                setTimeout(() => {
                    setIsLeaving(true);
                    setTimeout(() => {
                        // 回调父组件并传递用户信息，清理状态
                        props.onSuccess(result);
                        setIsSuccess(false);
                        setIsLeaving(false);
                        handleClose(); // 调用带动画的关闭
                    }, 300);
                }, 600);
            }
        } catch (err: any) {
            // 捕获后端错误或前端验证错误，显示在表单中
            setError(err.toString());
        } finally {
            // 无论成功失败，关闭加载状态
            setLoading(false);
        }
    };

    // ==================== 渲染逻辑 ====================

    return (
        // 条件渲染：仅当 props.show 为 true 时渲染模态框
        <Show when={props.show}>
            {/* 遮罩层：全屏半透明背景，点击关闭，控制进入/退出动画 */}
            <div
                classList={{ 
                    "modal-overlay": true, 
                    "overlay-out": isExiting()  // 退出动画类
                }}
                class="overlay-in"  // 进入动画类
                onClick={handleClose}
            >
                {/* 模态框内容容器：阻止冒泡防止点击关闭 */}
                <div
                    classList={{ 
                        "login-modal-content": true, 
                        "animate-out": isExiting()  // 退出动画类
                    }}
                    class="animate-in"  // 进入动画类
                    onClick={(e) => e.stopPropagation()}
                >
                    {/* 嵌套结构：外层遮罩 + 内容（注：此处存在嵌套冗余，实际应优化） */}
                    <div class="modal-overlay" onClick={props.onClose}>
                        <div class="login-modal-content" onClick={(e) => e.stopPropagation()}>

                            {/* ==================== 成功状态覆盖层 ==================== */}
                            <Show when={isSuccess()}>
                                <div classList={{
                                    'success-overlay': true,
                                    'leaving': isLeaving()  // 离开动画类
                                }}>
                                    {/* SVG 成功动画：圆形勾选图标 */}
                                    <div class="success-circle">
                                        <svg viewBox="0 0 52 52" class="checkmark">
                                            <circle class="checkmark__circle" cx="26" cy="26" r="25" fill="none" />
                                            <path class="checkmark__check" fill="none" d="M14.1 27.2l7.1 7.2 16.7-16.8" />
                                        </svg>
                                    </div>
                                    {/* 动态文本：注册成功/欢迎回来 */}
                                    <span class="success-text">
                                        {isRegister() ? '注册成功' : '欢迎回来'}
                                    </span>
                                </div>
                            </Show>

                            {/* ==================== 头部区域 ==================== */}
                            <div class="modal-header">
                                {/* 动态标题：根据模式切换 */}
                                <h3>{isRegister() ? '新用户注册' : '账号登录'}</h3>
                                {/* 关闭按钮：触发带动画的关闭 */}
                                <button class="close-btn" onClick={handleClose}>×</button>
                            </div>

                            {/* ==================== 表单区域 ==================== */}
                            <form class="login-form" onSubmit={handleSubmit}>
                                {/* 邮箱输入组 */}
                                <div class="input-group">
                                    <label>电子邮箱</label>
                                    <input
                                        type="email"
                                        value={email()}
                                        onInput={(e) => setEmail(e.currentTarget.value)}
                                        placeholder="example@mail.com"
                                        required  // HTML5 原生验证
                                    />
                                </div>
                                
                                {/* 密码输入组 */}
                                <div class="input-group">
                                    <label>密码</label>
                                    <input
                                        type="password"
                                        value={password()}
                                        onInput={(e) => setPassword(e.currentTarget.value)}
                                        placeholder="请输入密码"
                                        required
                                    />
                                </div>

                                {/* 条件渲染：确认密码（仅注册模式显示） */}
                                <Show when={isRegister()}>
                                    <div class="input-group">
                                        <label>确认密码</label>
                                        <input
                                            type="password"
                                            value={confirmPassword()}
                                            onInput={(e) => setConfirmPassword(e.currentTarget.value)}
                                            placeholder="请再次输入密码"
                                            required
                                        />
                                    </div>
                                </Show>

                                {/* 错误提示：仅当 error 非空时显示 */}
                                {error() && <div class="error-msg">{error()}</div>}

                                {/* 提交按钮：禁用状态由 loading 控制 */}
                                <button type="submit" class="login-submit-btn" disabled={loading()}>
                                    {loading() ? '请稍候...' : (isRegister() ? '跳转注册' : '立即登录')}
                                </button>
                            </form>

                            {/* ==================== 底部切换链接 ==================== */}
                            <div class="modal-footer">
                                <span>
                                    {isRegister() ? '已有账号？' : '没有账号？'}
                                    <a href="javascript:void(0)" onClick={toggleMode}>
                                        {isRegister() ? '立即登录' : '注册'}
                                    </a>
                                </span>
                            </div>
                        </div>
                    </div>
                </div>
            </div>
        </Show>
    );
};

// 默认导出组件
export default LoginModal;