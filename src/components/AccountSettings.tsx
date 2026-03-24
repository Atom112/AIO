import { Component } from 'solid-js';

/**
 * 账号设置页面组件
 * 
 * @component
 * @description 渲染用户账号信息展示页面，包含静态的用户信息卡片和退出登录按钮。
 *              当前实现为占位符版本，所有用户数据均为硬编码。
 * 
 * @returns {JSX.Element} 返回账号设置页面的 JSX 元素
 */
const AccountSettings: Component = () => {
    return (
        <div class="r w-full h-full animate-in fade-in slide-in-from-bottom-2 duration-400">
            <div class="w-full max-w-2xl border border-[var(--primary-color)] shadow-[inset_0_0_20px_1px_var(--primary-30)] rounded-xl bg-[var(--primary-5)] p-8">
                {/* 标题部分 */}
                <div class="border-b border-[var(--primary-20)] pb-3 mb-8">
                    <h3 class="text-xl font-bold tracking-tight text-white">账号信息</h3>
                </div>
                
                {/* 信息展示区 */}
                <div class="flex flex-col gap-4 mb-10">
                    <div class="flex flex-col gap-2.5">
                        <label class="text-xs font-bold text-[#666] uppercase tracking-widest">
                            当前登录身份
                        </label>
                        <div class="bg-[#121212] p-4 rounded-lg border border-[#333] text-[var(--primary-color)] font-mono text-lg shadow-inner flex items-center gap-3">
                            <span class="w-2 h-2 rounded-full bg-[var(--primary-color)] animate-pulse"></span>
                            Premium User
                        </div>
                    </div>
                </div>

                {/* 操作按钮 */}
                <div class="flex items-center gap-4">
                    <button 
                        class="bg-[#E08090] text-black border-none py-3 px-8 font-bold rounded-md cursor-pointer transition-all duration-200 hover:opacity-90 hover:scale-[1.02] active:scale-95 shadow-lg shadow-[#E08090]/10"
                    >
                        退出当前账号
                    </button>
                </div>
            </div>
        </div>
    );
};

export default AccountSettings;