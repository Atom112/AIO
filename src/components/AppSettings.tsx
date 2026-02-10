import { Component, createSignal } from 'solid-js';
import './AppSettings.css';
const AppSettings: Component = () => {
    const [autoStart, setAutoStart] = createSignal(true);

    return (
        <div class="tab-content-simple">
            <div class="placeholder-card">
                <h3>📱 应用信息</h3>
                <div class="setting-item">
                    <label>常规</label>
                    <div style="display: flex; align-items: center; gap: 10px; color: #eee; font-size: 14px;">
                        <span>开机自启</span>
                        <label class="switch">
                            <input type="checkbox" checked={autoStart()} onChange={(e) => setAutoStart(e.currentTarget.checked)} />
                            <span class="slider"></span>
                        </label>
                    </div>
                </div>
                <div class="setting-item">
                    <label>应用版本</label>
                    <div class="static-value">v1.2.5-stable</div>
                </div>
                <div class="setting-item">
                    <label>更新日志</label>
                    <div class="static-value" style="font-size: 12px; color: #999;">
                        - 优化了模型列表加载速度<br/>- 修复了侧边栏显示异常的问题
                    </div>
                </div>
            </div>
        </div>
    );
};

export default AppSettings;