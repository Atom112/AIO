import { createSignal, Component, Show, createEffect } from 'solid-js';
import { invoke } from '@tauri-apps/api/core';

interface LoginModalProps {
    show: boolean; // 控制模态框显示/隐藏
    onClose: () => void; // 关闭模态框回调
    onSuccess: (userData: any) => void; // 登录成功回调
}

/**
 * 登录/注册模态框组件
 * @param {LoginModalProps} props - 组件属性
 * @returns {JSX.Element} 登录模态框 JSX 元素
 */
const LoginModal: Component<LoginModalProps> = (props) => {

    const [isRegister, setIsRegister] = createSignal(false); // 当前模式：false=登录，true=注册
    const [email, setEmail] = createSignal(''); // 邮箱输入值
    const [username, setUsername] = createSignal(''); // 用户名（预留字段）
    const [password, setPassword] = createSignal(''); // 密码输入值
    const [loading, setLoading] = createSignal(false); // 提交加载状态
    const [error, setError] = createSignal(''); // 错误提示信息
    const [confirmPassword, setConfirmPassword] = createSignal(''); // 确认密码
    const [isSuccess, setIsSuccess] = createSignal(false); // 操作成功状态
    const [isLeaving, setIsLeaving] = createSignal(false); // 成功覆盖层离开动画状态
    const [isExiting, setIsExiting] = createSignal(false); // 模态框退出动画状态
    const [isEntering, setIsEntering] = createSignal(true); // 入场动画状态

    /**
     * 监听模态框显示状态，触发入场动画
     */
    createEffect(() => {
        if (props.show) {
            setIsEntering(true);
            setTimeout(() => setIsEntering(false), 0);
        }
    });

    /**
     * 切换登录/注册模式
     */
    const toggleMode = () => {
        setIsRegister(!isRegister());
        setError('');
        setPassword('');
        setConfirmPassword('');
    };

    /**
     * 处理关闭模态框（带退出动画）
     */
    const handleClose = () => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false);
            props.onClose();
        }, 300);
    };

    /**
     * 表单提交处理（登录或注册）
     * @param {Event} e - 表单提交事件
     */
    const handleSubmit = async (e: Event) => {
        e.preventDefault();
        setLoading(true);
        setError('');

        try {
            if (isRegister()) {
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
                    setIsLeaving(true);
                    setTimeout(() => {
                        setIsSuccess(false);
                        setIsLeaving(false);
                        setIsRegister(false);
                        setPassword('');
                        setConfirmPassword('');
                    }, 300);
                }, 800);

            } else {
                const result: any = await invoke('login_to_backend', {
                    username: email(),
                    password: password()
                });

                setIsSuccess(true);

                setTimeout(() => {
                    setIsLeaving(true);
                    setTimeout(() => {
                        props.onSuccess(result);
                        setIsSuccess(false);
                        setIsLeaving(false);
                        handleClose();
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
                classList={{
                    "opacity-0 pointer-events-none": isExiting() || isEntering(),
                    "opacity-100": !isExiting() && !isEntering()
                }}
                class="modal-overlay transition-all duration-200 ease-out"
                onClick={handleClose}
            >
                <div
                    classList={{
                        "scale-95 opacity-0": isExiting() || isEntering(),
                        "scale-100 opacity-100": !isExiting() && !isEntering()
                    }}
                    class="relative min-h-[400px] modal-panel w-[360px] p-6 text-[#e0e0e0] transition-all duration-500 ease-out transform pointer-events-auto"
                    onClick={(e) => e.stopPropagation()}
                >
                    <Show when={isSuccess()}>
                        <div
                            classList={{
                                'opacity-0 scale-105 blur-lg': isLeaving(),
                                'opacity-100 scale-100': !isLeaving()
                            }}
                            class="absolute inset-0 bg-dark/98 flex flex-col items-center justify-center z-50 rounded-lg transition-all duration-300"
                        >
                            <div>
                                <svg viewBox="0 0 52 52" class="w-[60px] h-[60px] rounded-full block stroke-[3] stroke-pri [stroke-miterlimit:10]">
                                    <circle class="checkmark__circle" cx="26" cy="26" r="25" fill="none" />
                                    <path class="checkmark__check" fill="none" d="M14.1 27.2l7.1 7.2 16.7-16.8" />
                                </svg>
                            </div>
                            <span class="mt-[15px] text-pri text-lg font-bold tracking-[2px]">
                                {isRegister() ? '注册成功' : '欢迎回来'}
                            </span>
                        </div>
                    </Show>

                    <div class="flex justify-between items-center mb-6">
                        <h3 class="m-0 text-lg text-pri font-medium">
                            {isRegister() ? '新用户注册' : '账号登录'}
                        </h3>
                        <button onClick={handleClose} class="close-btn">
                            &times;
                        </button>
                    </div>

                    <form class="flex flex-col gap-4" onSubmit={handleSubmit}>
                        <div class="flex flex-col gap-2">
                            <label class="text-[13px] text-[#888]">电子邮箱</label>
                            <input
                                type="email"
                                value={email()}
                                onInput={(e) => setEmail(e.currentTarget.value)}
                                placeholder="example@mail.com"
                                required
                                class="bg-dark-600 border border-dark-300 p-[10px] rounded-md text-white outline-none transition-colors focus:border-pri"
                            />
                        </div>

                        <div class="flex flex-col gap-2">
                            <label class="text-[13px] text-[#888]">密码</label>
                            <input
                                type="password"
                                value={password()}
                                onInput={(e) => setPassword(e.currentTarget.value)}
                                placeholder="请输入密码"
                                required
                                class="bg-dark-600 border border-dark-300 p-[10px] rounded-md text-white outline-none transition-colors focus:border-pri"
                            />
                        </div>

                        <div
                            class="overflow-hidden transition-[max-height,opacity,padding] duration-300"
                            style={{
                                "max-height": isRegister() ? '140px' : '0px',
                                "opacity": isRegister() ? '1' : '0',
                                "padding-top": isRegister() ? '0.5rem' : '0px',
                                "padding-bottom": isRegister() ? '0.5rem' : '0px'
                            }}
                        >
                            <div class="flex flex-col gap-2">
                                <label class="text-[13px] text-[#888]">确认密码</label>
                                <input
                                    type="password"
                                    value={confirmPassword()}
                                    onInput={(e) => setConfirmPassword(e.currentTarget.value)}
                                    placeholder="请再次输入密码"
                                    required={isRegister()}
                                    class="bg-dark-600 border border-dark-300 p-[10px] rounded-md text-white outline-none transition-colors focus:border-pri"
                                />
                            </div>
                        </div>

                        <Show when={error()}>
                            <div class="text-[#ff4d4f] text-[12px] mb-3 bg-[#fff2f0] p-2 rounded border border-[#ffccc7]">
                                {error()}
                            </div>
                        </Show>

                        <button
                            type="submit"
                            disabled={loading()}
                            class="w-full p-3 mt-[10px] bg-[rgba(var(--primary-rgb),0.2)] border border-pri text-pri rounded-md font-bold cursor-pointer transition-all hover:bg-pri hover:text-dark disabled:opacity-50 disabled:cursor-not-allowed"
                        >
                            {loading() ? '请稍候...' : (isRegister() ? '跳转注册' : '立即登录')}
                        </button>
                    </form>

                    <div class="mt-5 text-center text-[12px] text-[#888]">
                        <span>
                            {isRegister() ? '已有账号？' : '没有账号？'}
                            <a
                                href="javascript:void(0)"
                                onClick={toggleMode}
                                class="text-pri no-underline hover:underline ml-1"
                            >
                                {isRegister() ? '立即登录' : '注册'}
                            </a>
                        </span>
                    </div>
                </div>
            </div>
        </Show>
    );
}

export default LoginModal;