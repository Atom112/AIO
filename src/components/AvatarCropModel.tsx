/**
 * ============================================================================
 * 文件功能摘要
 * ============================================================================
 * 
 * 【核心功能】
 * 头像裁剪模态框组件，基于 Cropper.js 实现图片裁剪功能。
 * 提供圆形预览、缩放控制、裁剪保存等功能，支持进入/退出动画效果。
 * 
 * 【数据流流向】
 * ┌─────────────────────────────────────────────────────────────────────────┐
 * │  外部数据流入 (Props)                                                    │
 * │  ├── imageSrc: string ← 父组件传入的待裁剪图片 DataURL/URL              │
 * │  ├── onSave: (croppedDataUrl: string) => void ← 保存回调，传出裁剪结果   │
 * │  └── onCancel: () => void ← 取消回调，关闭模态框                         │
 * │                                                                          │
 * │  用户交互输出                                                            │
 * │  ├── cropper.zoomTo() → 实时缩放图片（Cropper.js 实例方法）              │
 * │  ├── cropper.getCroppedCanvas() → 获取裁剪后的 Canvas 对象               │
 * │  ├── canvas.toDataURL() → 压缩为 JPEG Base64（质量 0.8）                 │
 * │  └── onSave/onCancel → 通过 Props 回调通知父组件                         │
 * │                                                                          │
 * │  本地状态                                                                │
 * │  ├── zoomValue: 缩放滑块当前值（0.1-3）                                  │
 * │  └── isExiting: 控制退出动画状态                                         │
 * │                                                                          │
 * │  外部库                                                                  │
 * │  └── Cropper.js: 图片裁剪核心库，通过 ref 操作 DOM 实例                  │
 * └─────────────────────────────────────────────────────────────────────────┘
 * 
 * 【组件层级】
 * AvatarCropModal (本组件)
 * ├── 遮罩层 (点击关闭)
 * ├── 模态框容器
 * │   ├── 头部标题 + 关闭按钮
 * │   ├── 主区域
 * │   │   ├── 裁剪区 (img + Cropper.js 绑定)
 * │   │   └── 预览区 (圆形预览)
 * │   └── 底部控制区
 * │       ├── 缩放滑块
 * │       └── 操作按钮 (取消/保存)
 * ============================================================================
 */

// SolidJS 核心 API
import { Component, createSignal, onCleanup } from 'solid-js';
// Cropper.js: 图片裁剪库，提供拖拽、缩放、裁剪框等功能
import Cropper from 'cropperjs';
// Cropper.js 默认样式
import 'cropperjs/dist/cropper.css';
// 本地自定义样式
import './AvatarCropModel.css';

/**
 * 组件 Props 接口定义
 */
interface AvatarCropModalProps {
    /** 待裁剪的图片源，支持 DataURL 或远程 URL */
    imageSrc: string;
    /**
     * 保存回调函数
     * @param croppedDataUrl - 裁剪后的图片 Base64 DataURL (JPEG格式，256x256)
     */
    onSave: (croppedDataUrl: string) => void;
    /** 取消/关闭回调函数 */
    onCancel: () => void;
}

/**
 * 头像裁剪模态框组件
 * 
 * @component
 * @description 基于 Cropper.js 的图片裁剪弹窗，固定 1:1 比例输出 256x256 头像。
 *              支持缩放控制、实时预览、进入/退出动画。
 * 
 * @param {AvatarCropModalProps} props - 组件属性
 * @returns {JSX.Element} 裁剪模态框 JSX 元素
 */
const AvatarCropModal: Component<AvatarCropModalProps> = (props) => {
    // ==================== DOM 引用 ====================

    /** 图片元素引用：Cropper.js 绑定的目标元素 */
    let imageElement: HTMLImageElement | undefined;
    /** 预览容器引用：Cropper.js 的 preview 配置目标 */
    let previewElement: HTMLDivElement | undefined;
    /** Cropper.js 实例：通过 ref 保存以便调用实例方法 */
    let cropper: Cropper | null = null;

    // ==================== 本地状态 ====================

    /** 缩放值：范围 0.1-3，默认 1（原图大小），绑定滑块 */
    const [zoomValue, setZoomValue] = createSignal(1);
    /** 退出动画标记：true 时添加退出动画类名，动画完成后关闭 */
    const [isExiting, setIsExiting] = createSignal(false);

    // ==================== 动画控制 ====================

    /**
     * 触发退出动画并执行回调
     * 
     * 动画流程：
     * 1. 设置 isExiting=true，触发 CSS 退出动画（fadeOut/slideOut）
     * 2. 等待 250ms（与 CSS 动画时长一致）
     * 3. 重置 isExiting，执行业务回调（关闭或保存）
     * 
     * @param {() => void} callback - 动画结束后执行的回调函数
     */
    const triggerExit = (callback: () => void) => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false);
            callback(); // 动画结束后执行业务逻辑
        }, 250); // 这里的 250ms 应与 CSS 中的动画时间一致
    };

    /**
     * 处理关闭按钮点击
     * 先播放退出动画，再调用 onCancel 关闭模态框
     */
    const handleClose = () => {
        triggerExit(props.onCancel);
    };

    // ==================== Cropper.js 初始化 ====================

    /**
     * 初始化 Cropper.js 实例
     * 
     * 配置说明：
     * - aspectRatio: 1     固定 1:1 正方形裁剪（头像比例）
     * - viewMode: 1        限制裁剪框不超过画布
     * - dragMode: 'move'   拖拽模式为移动图片（而非裁剪框）
     * - guides: false      隐藏网格参考线
     * - center: true       显示中心指示器
     * - cropBoxMovable: true    允许移动裁剪框
     * - cropBoxResizable: true  允许调整裁剪框大小
     * - preview: 绑定预览容器，实现实时预览
     */
    const initCropper = () => {
        // 防御性检查：确保 DOM 元素已挂载
        if (!imageElement) return;

        // 销毁已有实例，防止内存泄漏和重复初始化
        if (cropper) {
            cropper.destroy();
        }

        // 创建 Cropper.js 实例
        cropper = new Cropper(imageElement, {
            aspectRatio: 1,           // 固定正方形比例
            viewMode: 1,              // 视图模式：限制裁剪框
            dragMode: 'move',         // 拖拽模式：移动图片
            guides: false,            // 隐藏裁剪网格
            center: true,             // 显示中心点
            highlight: false,         // 不显示高亮区域
            cropBoxMovable: true,     // 可移动裁剪框
            cropBoxResizable: true,   // 可调整裁剪框
            toggleDragModeOnDblclick: false, // 禁用双击切换拖拽模式
            preview: previewElement,  // 实时预览目标容器
            
            /**
             * Cropper 准备就绪回调
             * 重置缩放值为 1，确保滑块与实例状态同步
             */
            ready() {
                setZoomValue(1);
            }
        });
    };

    /**
     * 组件卸载清理：销毁 Cropper.js 实例，释放内存
     */
    onCleanup(() => {
        cropper?.destroy();
    });

    // ==================== 业务操作 ====================

    /**
     * 处理保存按钮点击
     * 
     * 数据流：
     * 1. 调用 cropper.getCroppedCanvas() 获取裁剪后的 Canvas
     *    - 固定输出 256x256 像素
     *    - 启用高质量图像平滑
     * 2. 使用 canvas.toDataURL() 压缩为 JPEG，质量 0.8（80%）
     * 3. 触发退出动画，动画完成后通过 onSave 回调传出结果
     */
    const handleSave = () => {
        if (cropper) {
            // 获取裁剪后的 Canvas 对象，指定输出尺寸
            const canvas = cropper.getCroppedCanvas({
                width: 256,               // 输出宽度：256px
                height: 256,              // 输出高度：256px
                imageSmoothingEnabled: true,   // 启用图像平滑
                imageSmoothingQuality: 'high', // 高质量平滑
            });

            // 压缩为 JPEG Base64，质量 0.8（平衡画质与体积）
            const compressedBase64 = canvas.toDataURL('image/jpeg', 0.8);
            
            // 先播放退出动画，再回调保存
            triggerExit(() => {
                props.onSave(compressedBase64);
            });
        }
    };

    // ==================== 渲染逻辑 ====================

    return (
        // 遮罩层：全屏半透明背景，点击关闭
        <div 
            classList={{ 
                "crop-modal-overlay": true, 
                "overlay-out": isExiting()  // 退出动画类
            }} 
            class="overlay-in"  // 进入动画类
        >
            {/* 模态框容器：包含所有内容 */}
            <div
                classList={{ 
                    "crop-modal-container": true, 
                    "animate-out": isExiting()  // 退出动画类
                }}
                class="animate-in"  // 进入动画类
            >
                {/* 头部：标题 + 关闭按钮 */}
                <div class="crop-modal-header">
                    <span>裁剪图片</span>
                    <button class="close-btn" onClick={handleClose}>✕</button>
                </div>

                {/* 主区域：裁剪区 + 预览区 */}
                <div class="crop-main-area">
                    {/* 左侧：图片裁剪区 */}
                    <div class="cropper-wrapper">
                        <img
                            ref={imageElement}           // DOM 引用绑定
                            src={props.imageSrc}         // 图片源
                            onLoad={initCropper}         // 图片加载完成后初始化 Cropper
                            crossOrigin="anonymous"      // 允许跨域图片处理
                            style={{ 
                                "display": "block",      // 消除图片底部间隙
                                "max-width": "100%"      // 响应式宽度
                            }}
                        />
                    </div>

                    {/* 右侧：圆形预览区 */}
                    <div class="crop-preview-side">
                        <div class="preview-label">预览</div>
                        {/* previewElement 引用：Cropper.js 自动渲染预览到此容器 */}
                        <div ref={previewElement} class="avatar-preview-circle"></div>
                    </div>
                </div>

                {/* 底部控制区：缩放滑块 + 操作按钮 */}
                <div class="crop-controls">
                    {/* 缩放控制 */}
                    <div class="zoom-slider-container">
                        <input
                            type="range"
                            min="0.1"    // 最小缩放：10%
                            max="3"      // 最大缩放：300%
                            step="0.05"  // 步进：5%
                            value={zoomValue()}  // 双向绑定缩放值
                            onInput={(e) => {
                                const val = parseFloat(e.currentTarget.value);
                                // 调用 Cropper.js 实例方法实时缩放
                                cropper?.zoomTo(val);
                                // 同步更新本地状态
                                setZoomValue(val);
                            }}
                        />
                    </div>

                    {/* 操作按钮组 */}
                    <div class="modal-actions">
                        <button class="btn-cancel" onClick={handleClose}>取消</button>
                        <button class="btn-save" onClick={handleSave}>保存头像</button>
                    </div>
                </div>
            </div>
        </div>
    );
};

// 默认导出组件
export default AvatarCropModal;