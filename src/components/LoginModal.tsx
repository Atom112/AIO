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

    const toggleMode = () => {
        setIsRegister(!isRegister());
        setError('');
        setPassword('');
        setConfirmPassword('');
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
                alert("注册成功，请登录！");
                setIsRegister(false); // 注册成功后跳回登录
            } else {
                // 登录逻辑
                const result: any = await invoke('login_to_backend', { 
                    username: email(), 
                    password: password() 
                });
                props.onSuccess(result);
                props.onClose();
            }
        } catch (err: any) {
            setError(err.toString());
        } finally {
            setLoading(false);
        }
    };

    return (
        <Show when={props.show}>
            <div class="modal-overlay" onClick={props.onClose}>
                <div class="login-modal-content" onClick={(e) => e.stopPropagation()}>
                    <div class="modal-header">
                        <h3>{isRegister() ? '新用户注册' : '账号登录'}</h3>
                        <button class="close-btn" onClick={props.onClose}>×</button>
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
        </Show>
    );
};

export default LoginModal;