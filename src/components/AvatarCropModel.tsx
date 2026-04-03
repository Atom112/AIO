import { Component, createSignal, onCleanup, onMount } from 'solid-js';
import Cropper from 'cropperjs';
import 'cropperjs/dist/cropper.css';

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

    /** 图片元素引用：Cropper.js 绑定的目标元素 */
    let imageElement: HTMLImageElement | undefined;
    /** 预览容器引用：Cropper.js 的 preview 配置目标 */
    let previewElement: HTMLDivElement | undefined;
    /** Cropper.js 实例：通过 ref 保存以便调用实例方法 */
    let cropper: Cropper | null = null;

    /** 缩放值：范围 0.1-3，默认 1（原图大小），绑定滑块 */
    const [zoomValue, setZoomValue] = createSignal(1);
    /** 退出动画标记：true 时添加退出动画类名，动画完成后关闭 */
    const [isExiting, setIsExiting] = createSignal(false);
    /** 入场动画标记：true 时显示初始进入状态 */
    const [isEntering, setIsEntering] = createSignal(true);

    // 入场动画：组件挂载后立即从隐藏状态过渡到可见状态
    onMount(() => {
        setIsEntering(true);
        const enterTimer = setTimeout(() => setIsEntering(false), 20); // 20ms 触发样式变更
        onCleanup(() => clearTimeout(enterTimer));
    });

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

    return (
        <div
            classList={{
                'opacity-0 pointer-events-none': isExiting() || isEntering(),
                'opacity-100': !isExiting() && !isEntering(),
            }}
            class="modal-overlay bg-black/85 z-[1000] transition-all duration-300 ease-out"
            onClick={(e) => e.target === e.currentTarget && handleClose()}
        >
            <div
                classList={{
                    'scale-95 opacity-0 translate-y-2': isExiting() || isEntering(),
                    'scale-100 opacity-100 translate-y-0': !isExiting() && !isEntering(),
                }}
                class="modal-panel bg-dark-850 w-[600px] overflow-hidden flex flex-col transition-all duration-250 ease-out transform"
            >
                <div class="flex items-center justify-between px-[20px] py-[15px] border-b border-dark-300 text-pri font-bold">
                    <span>裁剪图片</span>
                    <button onClick={handleClose} class="close-btn">
                        &times;
                    </button>
                </div>

                <div class="flex h-[350px] p-[20px] gap-[20px]">
                    <div class="flex-1 bg-black rounded-md overflow-hidden border border-dark-300">
                        <img
                            ref={imageElement}
                            src={props.imageSrc}
                            onLoad={initCropper}
                            crossOrigin="anonymous"
                            style={{
                                "display": "block",
                                "max-width": "100%"
                            }}
                        />
                    </div>

                    <div class="w-[150px] flex flex-col items-center justify-center">
                        <div class="text-[#888] text-[12px] mb-[10px]">预览</div>
                        <div ref={previewElement} class="w-[120px] h-[120px] rounded-full overflow-hidden border-2 border-pri bg-black"></div>
                    </div>
                </div>

                <div class="p-[20px] bg-dark-700">
                    <div class="flex items-center gap-[10px] mb-[20px]">
                        <input
                            type="range"
                            min="0.1"
                            max="3"
                            step="0.05"
                            class="flex-1 accent-pri cursor-pointer bg-pri rounded-[4px] h-[6px] transition-colors duration-300"
                            value={zoomValue()}
                            onInput={(e) => {
                                const val = parseFloat(e.currentTarget.value);
                                cropper?.zoomTo(val);
                                setZoomValue(val);
                            }}
                        />
                    </div>

                    <div class="flex justify-end gap-3">
                        <button onClick={handleClose} class="px-5 py-2.5 border-0 cursor-pointer font-bold bg-dark-100 text-[#e0e0e0] rounded-lg transition-all duration-200 hover:bg-dark-50">
                            取消
                        </button>
                        <button onClick={handleSave} class="px-5 py-2.5 border-0 cursor-pointer font-bold bg-pri text-black rounded-lg hover:scale-105 transition-all duration-200">
                            保存
                        </button>
                    </div>
                </div>
            </div>
        </div>
    );
};

export default AvatarCropModal;