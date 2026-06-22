import { Component } from 'solid-js';

const AccountSettings: Component = () => {
    return (
        <div class="w-full h-full">
            <div class="w-full max-w-2xl rounded-xl p-8"
                 style="background: rgba(18, 22, 35, 0.6); backdrop-filter: blur(30px); border: 1px solid rgba(255, 255, 255, 0.06); box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);">
                <div class="pb-3 mb-8" style="border-bottom: 1px solid rgba(255,255,255,0.06);">
                    <h3 class="text-xl font-bold tracking-tight" style="color: rgba(255,255,255,0.85);">账号信息</h3>
                </div>

                <div class="flex flex-col gap-4 mb-10">
                    <div class="flex flex-col gap-2.5">
                        <label class="text-xs font-bold uppercase tracking-widest" style="color: rgba(255,255,255,0.3);">
                            当前登录身份
                        </label>
                        <div class="p-4 rounded-lg text-lg flex items-center gap-3"
                             style="background: rgba(0,0,0,0.25); border: 1px solid rgba(255,255,255,0.06); color: rgba(124,154,191,0.7); font-family: monospace;">
                            <span class="w-2 h-2 rounded-full" style="background: rgba(124,154,191,0.5); animation: pulse 2s infinite;"></span>
                            Premium User
                        </div>
                    </div>
                </div>

                <div class="flex items-center gap-4">
                    <button
                        class="border-none py-3 px-8 font-bold rounded-md cursor-pointer transition-all duration-200 hover:opacity-90 hover:scale-[1.02] active:scale-95"
                        style="background: rgba(224,128,144,0.2); color: rgba(224,128,144,0.8);">
                        退出当前账号
                    </button>
                </div>
            </div>
        </div>
    );
};

export default AccountSettings;
