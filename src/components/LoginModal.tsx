import { createSignal, Component, Show } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';
import './LoginModal.css';

interface LoginModalProps {
    show: boolean;
    onClose: () => void;
    onSuccess: (userData: any) => void;
}

const LoginModal: Component<LoginModalProps> = (props) => {
    const [isRegister, setIsRegister] = createSignal(false);
    const [email, setEmail] = createSignal('');
    const [username, setUsername] = createSignal('');
    const [password, setPassword] = createSignal('');
    const [loading, setLoading] = createSignal(false);
    const [error, setError] = createSignal('');
    const [confirmPassword, setConfirmPassword] = createSignal('');
    const [isSuccess, setIsSuccess] = createSignal(false);
    const [isLeaving, setIsLeaving] = createSignal(false); // 新增：控制退出动画
    const [isExiting, setIsExiting] = createSignal(false);

    const toggleMode = () => {
        setIsRegister(!isRegister());
        setError('');
        setPassword('');
        setConfirmPassword('');
    };

    const handleClose = () => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false); // 重置状态
            props.onClose();     // 真正销毁组件
        }, 200); // 对应 CSS 动画时长
    };

    const handleSubmit = async (e: Event) => {
        e.preventDefault();
        setLoading(true);
        setError('');

        try {
            if (isRegister()) {
                // 注册逻辑
                if (password() !== confirmPassword()) {
                    throw new Error("两次输入的密码不一致");
                }
                await invoke('register_to_backend', {
                    email: email(),
                    password: password(),
                    confirmPassword: confirmPassword()
                });
                setIsSuccess(true);
                setTimeout(() => {
                    setIsLeaving(true); // 开启淡出动画
                    setTimeout(() => {
                        setIsSuccess(false);
                        setIsLeaving(false); // 重置状态
                        setIsRegister(false);
                        setPassword('');
                        setConfirmPassword('');
                    }, 300); // 这里的 500ms 对应 CSS 动画时长
                }, 800);
            } else {
                // 登录逻辑
                const result: any = await invoke('login_to_backend', {
                    username: email(),
                    password: password()
                });
                // 登录成功视觉反馈
                setIsSuccess(true);
                setTimeout(() => {
                    setIsLeaving(true);
                    setTimeout(() => {
                        props.onSuccess(result);
                        setIsSuccess(false);
                        setIsLeaving(false);
                        handleClose(); // 调用带动画的关闭
                    }, 300);
                }, 600);
            }
        } catch (err: any) {
            setError(err.toString());
        } finally {
            setLoading(false);
        }
    };

    return (
        <Show when={props.show}>
            <div
                classList={{ "modal-overlay": true, "overlay-out": isExiting() }}
                class="overlay-in"
                onClick={handleClose}
            >
                <div
                    classList={{ "login-modal-content": true, "animate-out": isExiting() }}
                    class="animate-in"
                    onClick={(e) => e.stopPropagation()}
                >
                    <div class="modal-overlay" onClick={props.onClose}>
                        <div class="login-modal-content" onClick={(e) => e.stopPropagation()}>

                            {/* --- 新增：视觉反馈层 --- */}
                            <Show when={isSuccess()}>
                                <div classList={{
                                    'success-overlay': true,
                                    'leaving': isLeaving() // 当正在离开时应用该类
                                }}>
                                    <div class="success-circle">
                                        <svg viewBox="0 0 52 52" class="checkmark">
                                            <circle class="checkmark__circle" cx="26" cy="26" r="25" fill="none" />
                                            <path class="checkmark__check" fill="none" d="M14.1 27.2l7.1 7.2 16.7-16.8" />
                                        </svg>
                                    </div>
                                    <span class="success-text">{isRegister() ? '注册成功' : '欢迎回来'}</span>
                                </div>
                            </Show>

                            <div class="modal-header">
                                <h3>{isRegister() ? '新用户注册' : '账号登录'}</h3>
                                <button class="close-btn" onClick={handleClose}>×</button>
                            </div>

                            <form class="login-form" onSubmit={handleSubmit}>
                                <div class="input-group">
                                    <label>电子邮箱</label>
                                    <input
                                        type="email"
                                        value={email()}
                                        onInput={(e) => setEmail(e.currentTarget.value)}
                                        placeholder="example@mail.com"
                                        required
                                    />
                                </div>
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

                                {/* 仅在注册模式下显示 */}
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

                                {error() && <div class="error-msg">{error()}</div>}

                                <button type="submit" class="login-submit-btn" disabled={loading()}>
                                    {loading() ? '请稍候...' : (isRegister() ? '跳转注册' : '立即登录')}
                                </button>
                            </form>

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

export default LoginModal;