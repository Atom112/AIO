import { Component } from 'solid-js';
import './AccountSettings.css';

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
                <div class="setting-item">
                    <label>当前用户</label>
                    <div class="static-value">Premium User</div>
                </div>

                <div class="setting-item">
                    <label>注册邮箱</label>
                    <div class="static-value">user@example.com</div>
                </div>

                <div class="setting-item">
                    <label>订阅方案</label>
                    {/* 硬编码的订阅信息：专业版年度订阅 */}
                    <div class="static-value">专业版 (按年订阅)</div>
                </div>

                <button 
                    class="save-settings-button" 
                    style="width: 150px; margin-top: 20px;"
                >
                    退出登录
                </button>
            </div>
        </div>
    );
};

export default AccountSettings;