// src/components/AvatarCropModal.tsx
import { Component, onMount, createSignal, onCleanup } from 'solid-js';
import Cropper from 'cropperjs';
import 'cropperjs/dist/cropper.css';
import './AvatarCropModel.css';

interface AvatarCropModalProps {
    imageSrc: string; // 选中的原始图片路径（Blob URL）
    onSave: (croppedDataUrl: string) => void;
    onCancel: () => void;
}

const AvatarCropModal: Component<AvatarCropModalProps> = (props) => {
    let imageElement: HTMLImageElement | undefined;
    let previewElement: HTMLDivElement | undefined;
    let cropper: Cropper | null = null;
    const [zoomValue, setZoomValue] = createSignal(1);

    onMount(() => {
        if (imageElement) {
            cropper = new Cropper(imageElement, {
                aspectRatio: 1, // 强制正方形裁剪
                viewMode: 1,
                dragMode: 'move',
                guides: false,
                center: true,
                highlight: false,
                cropBoxMovable: true,
                cropBoxResizable: true,
                toggleDragModeOnDblclick: false,
                preview: previewElement, // 绑定预览框
                ready() {
                    // 初始化缩放
                    setZoomValue(1);
                }
            });
        }
    });

    onCleanup(() => {
        cropper?.destroy();
    });

    const handleSave = () => {
        if (cropper) {
            // 获取输出的 Canvas，设定最终头像分辨率 256x256
            const canvas = cropper.getCroppedCanvas({
                width: 256,
                height: 256,
                imageSmoothingEnabled: true,
                imageSmoothingQuality: 'high',
            });
            props.onSave(canvas.toDataURL('image/png'));
        }
    };

    return (
        <div class="crop-modal-overlay">
            <div class="crop-modal-container">
                <div class="crop-modal-header">
                    <span>裁剪图片</span>
                    <button class="close-btn" onClick={props.onCancel}>✕</button>
                </div>

                <div class="crop-main-area">
                    <div class="cropper-wrapper">
                        <img ref={imageElement} src={props.imageSrc} style={{ "max-width": "100%" }} />
                    </div>

                    <div class="crop-preview-side">
                        <div class="preview-label">预览</div>
                        <div ref={previewElement} class="avatar-preview-circle"></div>
                    </div>
                </div>

                <div class="crop-controls">
                    <div class="zoom-slider-container">
                        <svg fill="none" viewBox="0 0 24 24" stroke="currentColor" class="zoom-icon"><path d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0zM10 7v3m0 0v3m0-3h3m-3 0H7" /></svg>
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
                        <svg fill="none" viewBox="0 0 24 24" stroke="currentColor" class="zoom-icon"><path d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0zM13 10H7" /></svg>
                    </div>

                    <div class="modal-actions">
                        <button class="btn-cancel" onClick={props.onCancel}>取消</button>
                        <button class="btn-save" onClick={handleSave}>保存头像</button>
                    </div>
                </div>
            </div>
        </div>
    );
};

export default AvatarCropModal;