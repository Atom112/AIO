// src/components/AvatarCropModel.tsx

import { Component, onMount, createSignal, onCleanup } from 'solid-js';
import Cropper from 'cropperjs';
import 'cropperjs/dist/cropper.css';
import './AvatarCropModel.css';

interface AvatarCropModalProps {
    imageSrc: string;
    onSave: (croppedDataUrl: string) => void;
    onCancel: () => void;
}

const AvatarCropModal: Component<AvatarCropModalProps> = (props) => {
    let imageElement: HTMLImageElement | undefined;
    let previewElement: HTMLDivElement | undefined;
    let cropper: Cropper | null = null;
    const [zoomValue, setZoomValue] = createSignal(1);


    const [isExiting, setIsExiting] = createSignal(false);

    const handleClose = () => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false);
            props.onCancel();
        }, 250);
    };


    // --- 核心修复：定义初始化函数 ---
    const initCropper = () => {
        if (!imageElement) return;

        // 如果已经存在之前的实例，先销毁（防止重复初始化）
        if (cropper) {
            cropper.destroy();
        }

        // 确保图片加载完成后再初始化，Cropper 需要图片的真实宽高
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

    onCleanup(() => {
        cropper?.destroy();
    });

    const handleSave = () => {
        if (cropper) {
            const canvas = cropper.getCroppedCanvas({
                width: 256,
                height: 256,
                imageSmoothingEnabled: true,
                imageSmoothingQuality: 'high',
            });
            const compressedBase64 = canvas.toDataURL('image/jpeg', 0.8);

            props.onSave(compressedBase64);
        }
    };

    return (
        <div classList={{ "crop-modal-overlay": true, "overlay-out": isExiting() }} class="overlay-in">
            <div
                classList={{ "crop-modal-container": true, "animate-out": isExiting() }}
                class="animate-in"
            >
                <div class="crop-modal-header">
                    <span>裁剪图片</span>
                    <button class="close-btn" onClick={handleClose}>✕</button>
                </div>

                <div class="crop-main-area">
                    <div class="cropper-wrapper">
                        {/* 
                            关键：
                            1. 增加 onLoad 事件，确保图片加载后再初始化 Cropper
                            2. 增加 crossOrigin 属性，避免 Canvas 跨域污染
                            3. 加入 display: block 确保 Cropper 工作正常
                        */}
                        <img
                            ref={imageElement}
                            src={props.imageSrc}
                            onLoad={initCropper}
                            crossOrigin="anonymous"
                            style={{ "display": "block", "max-width": "100%" }}
                        />
                    </div>

                    <div class="crop-preview-side">
                        <div class="preview-label">预览</div>
                        <div ref={previewElement} class="avatar-preview-circle"></div>
                    </div>
                </div>

                <div class="crop-controls">
                    <div class="zoom-slider-container">
                        <input
                            type="range"
                            min="0.1"
                            max="3"
                            step="0.05"
                            value={zoomValue()}
                            onInput={(e) => {
                                const val = parseFloat(e.currentTarget.value);
                                cropper?.zoomTo(val);
                                setZoomValue(val);
                            }}
                        />
                    </div>

                    <div class="modal-actions">
                        <button class="btn-cancel" onClick={handleClose}>取消</button>
                        <button class="btn-save" onClick={handleSave}>保存头像</button>
                    </div>
                </div>
            </div>
        </div>
    );
};

export default AvatarCropModal;