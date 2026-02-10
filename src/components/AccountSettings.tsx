import { Component } from 'solid-js';
import './AccountSettings.css';
const AccountSettings: Component = () => {
    return (
        <div class="tab-content-simple">
            <div class="placeholder-card">
                <h3>👤 账号信息</h3>
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
                    <div class="static-value" style="color: #08ddf9;">专业版 (按年订阅)</div>
                </div>
                <button class="save-settings-button" style="width: 150px; margin-top: 20px;">退出登录</button>
            </div>
        </div>
    );
};

export default AccountSettings;