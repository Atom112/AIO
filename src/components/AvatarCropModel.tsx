// src/components/AvatarCropModel.tsx

import { Component, createSignal, onCleanup } from 'solid-js';
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

    const triggerExit = (callback: () => void) => {
        setIsExiting(true);
        setTimeout(() => {
            setIsExiting(false);
            callback(); // 动画结束后执行业务逻辑
        }, 250); // 这里的 250ms 应与 CSS 中的动画时间一致
    };


    const handleClose = () => {
        triggerExit(props.onCancel);
    };

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
            triggerExit(() => {
                props.onSave(compressedBase64);
            });

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