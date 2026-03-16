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
        <div class="tab-content-simple">
            <div class="placeholder-card">
                <h3>账号信息</h3>
                <div class="flex flex-col gap-2">
                    <label>
                        当前用户
                        </label>
                    <div class="static-value">Premium User</div>
                </div>

                <button 
                    class="mt-[10px] bg-[var(--primary-color)] text-black border-none p-[12px] font-bold rounded-[6px] cursor-pointer transition-opacity duration-200 hover:opacity-80"
                    style="width: 150px; margin-top: 20px;"
                >
                    退出登录
                </button>
            </div>
        </div>
    );
};

export default AccountSettings;