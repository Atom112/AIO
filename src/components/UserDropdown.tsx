import { Component, Show, createSignal, createEffect } from 'solid-js';
import Icon from './Icon';

interface UserDropdownProps {
  avatar: string;
  isLoggedIn: boolean;
  onEditAvatar: () => void;
  onLoginClick: () => void;
  onLogout: () => void;
}

const UserDropdown: Component<UserDropdownProps> = (props) => {
  const [isVisible, setIsVisible] = createSignal(false);
  const [imgSrc, setImgSrc] = createSignal(props.avatar);
  const [isLoaded, setIsLoaded] = createSignal(false);

  createEffect(() => { setImgSrc(props.avatar); setIsLoaded(false); });

  return (
    <div class="relative flex items-center cursor-pointer [app-region:no-drag]"
      onMouseEnter={() => setIsVisible(true)}
      onMouseLeave={() => setIsVisible(false)}>
      <div class="relative w-10 h-10 rounded-full overflow-hidden transition-all duration-200"
        style="border: 2px solid rgba(255,255,255,0.06);"
        onMouseEnter={(e) => e.currentTarget.style.borderColor = 'rgba(124,154,191,0.3)'}
        onMouseLeave={(e) => e.currentTarget.style.borderColor = 'rgba(255,255,255,0.06)'}>
        <img src={imgSrc()} alt="User Avatar" class="w-full h-full object-cover transition-opacity duration-300"
          classList={{ 'opacity-0': !isLoaded(), 'opacity-100': isLoaded() }}
          onLoad={() => setIsLoaded(true)}
          onError={() => setImgSrc('/icons/app-logo/user.svg')} />
        <div class="absolute inset-0 w-full h-full flex items-center justify-center transition-opacity duration-300 pointer-events-none"
          style="background: rgba(255,255,255,0.04);"
          classList={{ 'opacity-100': !isLoaded(), 'opacity-0': isLoaded() }}>
          <Icon src="/icons/app-logo/user.svg" class="w-5 h-5 opacity-50" />
        </div>
      </div>
      <div
        class="absolute top-full left-1/2 -translate-x-1/2 mt-3 min-w-[140px] rounded-lg shadow-[0_4px_15px_rgba(0,0,0,0.4)] z-[1000] transition-all duration-200 p-1.5"
        classList={{ 'invisible opacity-0': !isVisible(), 'visible opacity-100': isVisible() }}
        style="background: rgba(18, 22, 35, 0.85); backdrop-filter: blur(30px); -webkit-backdrop-filter: blur(30px); border: 1px solid rgba(255, 255, 255, 0.06);">
        <div class="user-menu-item" onClick={props.onEditAvatar}>
          <Icon src="/icons/app-logo/camera.svg" class="w-4 h-4" />
          更换头像
        </div>
        <Show when={props.isLoggedIn}
          fallback={<div class="user-menu-item" onClick={props.onLoginClick}><Icon src="/icons/app-logo/user-profile.svg" class="w-4 h-4" />登录账号</div>}>
          <div class="user-menu-item" style="color: rgba(224,128,144,0.8);"
            onMouseEnter={(e) => e.currentTarget.style.background = 'rgba(224,128,144,0.1)'}
            onMouseLeave={(e) => e.currentTarget.style.background = 'transparent'}
            onClick={props.onLogout}>
            <Icon src="/icons/app-logo/logout.svg" class="w-4 h-4" />
            退出登录
          </div>
        </Show>
      </div>
    </div>
  );
};

export default UserDropdown;
