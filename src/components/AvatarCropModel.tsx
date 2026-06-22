import { Component, createSignal, onCleanup, onMount } from 'solid-js';
import Cropper from 'cropperjs';
import 'cropperjs/dist/cropper.css';

interface AvatarCropModalProps {
    imageSrc: string; // 待裁剪的图片源
    onSave: (croppedDataUrl: string) => void; // 保存回调，返回裁剪后的 JPEG Base64
    onCancel: () => void; // 取消/关闭回调
}

/**
 * 头像裁剪模态框组件
 * @param {AvatarCropModalProps} props - 组件属性
 * @returns {JSX.Element} 裁剪模态框 JSX 元素
 */
const AvatarCropModal: Component<AvatarCropModalProps> = (props) => {

    let imageElement: HTMLImageElement | undefined; // 图片元素引用
    let previewElement: HTMLDivElement | undefined; // 预览容器引用
    let cropper: Cropper | null = null; // Cropper.js 实例

    const [zoomValue, setZoomValue] = createSignal(1); // 缩放值 (0.1-3)
    const [isExiting, setIsExiting] = createSignal(false); // 退出动画标记
    const [isEntering, setIsEntering] = createSignal(true); // 进入动画标记

    onMount(() => {
        setIsEntering(true);
        const enterTimer = setTimeout(() => setIsEntering(false), 20);
        onCleanup(() => clearTimeout(enterTimer));
    });

    /**
     * 触发退出动画并执行回调
     * @param {() => void} callback - 动画结束后执行的回调函数
     */
    const triggerExit = (callback: () => void) => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false);
            callback();
        }, 250);
    };

    /**
     * 处理关闭按钮点击
     */
    const handleClose = () => {
        triggerExit(props.onCancel);
    };

    /**
     * 初始化 Cropper.js 实例
     */
    const initCropper = () => {
        if (!imageElement) return;

        if (cropper) {
            cropper.destroy();
        }

        cropper = new Cropper(imageElement, {
            aspectRatio: 1,
            viewMode: 1,
            dragMode: 'move',
            guides: false,
            center: true,
            highlight: false,
            cropBoxMovable: true,
            cropBoxResizable: true,
            toggleDragModeOnDblclick: false,
            preview: previewElement,

            ready() {
                setZoomValue(1);
            }
        });
    };

    /**
     * 组件卸载时销毁 Cropper.js 实例
     */
    onCleanup(() => {
        cropper?.destroy();
    });

    /**
     * 处理保存按钮点击
     */
    const handleSave = () => {
        if (cropper) {
            const canvas = cropper.getCroppedCanvas({
                width: 256,
                height: 256,
                imageSmoothingEnabled: true,
                imageSmoothingQuality: 'high',
            });

            const compressedBase64 = canvas.toDataURL('image/jpeg', 0.8);

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