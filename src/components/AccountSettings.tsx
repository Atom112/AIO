/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * 账号设置页面组件，展示当前用户的账号基本信息，包括用户名、注册邮箱、
 * 订阅方案等静态信息，并提供退出登录按钮。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  数据方向: 纯展示组件，无外部数据流入                                     │
 * │  本地文件: 导入 AccountSettings.css 样式文件                             │
 * │  网络请求: 无                                                            │
 * │  状态管理: 无（静态数据硬编码）                                           │
 * │  用户交互: 退出登录按钮（当前为UI占位，无实际逻辑）                        │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * AccountSettings (本组件) → 纯展示型子组件
 * 
 * ============================================================================
 */

// 导入 SolidJS 的 Component 类型，用于定义函数式组件
import { Component } from 'solid-js';
// 导入本地 CSS 样式文件，作用于本组件的样式定义
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
        // 主容器：采用 tab-content-simple 类名，用于标签页内容区域的基础布局
        <div class="tab-content-simple">
            {/* 
                信息卡片容器：placeholder-card 类名提供卡片式UI样式
                包含所有账号相关信息的展示
            */}
            <div class="placeholder-card">
                {/* 页面标题：使用表情符号增强视觉识别度 */}
                <h3>👤 账号信息</h3>
                
                {/* 
                    设置项 1: 当前用户
                    setting-item 类名定义了标签-值对的布局结构
                */}
                <div class="setting-item">
                    {/* 标签说明 */}
                    <label>当前用户</label>
                    {/* 静态值展示：实际应用中应从用户状态或API获取 */}
                    <div class="static-value">Premium User</div>
                </div>

                {/* 
                    设置项 2: 注册邮箱
                    当前为硬编码的示例数据
                */}
                <div class="setting-item">
                    <label>注册邮箱</label>
                    {/* 示例邮箱地址，生产环境应动态渲染 */}
                    <div class="static-value">user@example.com</div>
                </div>

                {/* 
                    设置项 3: 订阅方案
                    展示用户的付费订阅信息
                */}
                <div class="setting-item">
                    <label>订阅方案</label>
                    {/* 硬编码的订阅信息：专业版年度订阅 */}
                    <div class="static-value">专业版 (按年订阅)</div>
                </div>

                {/* 
                    退出登录按钮
                    TODO: 需要绑定实际的登出逻辑（如清除token、重置全局状态、跳转登录页等）
                    当前样式：固定宽度150px，顶部外边距20px实现与其他元素的间距
                */}
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

// 默认导出：使其他模块可通过 import AccountSettings from './AccountSettings' 引入
export default AccountSettings;