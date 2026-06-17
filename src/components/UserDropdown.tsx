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

  createEffect(() => {
    setImgSrc(props.avatar);
    setIsLoaded(false);
  });

  return (
    <div
      class="relative flex items-center cursor-pointer [app-region:no-drag]"
      onMouseEnter={() => setIsVisible(true)}
      onMouseLeave={() => setIsVisible(false)}
    >
      <div class="relative w-10 h-10 rounded-full overflow-hidden border-2 border-dark-300 transition-all duration-200 hover:border-pri">
        <img
          src={imgSrc()}
          alt="User Avatar"
          class="w-full h-full object-cover transition-opacity duration-300"
          classList={{ 'opacity-0': !isLoaded(), 'opacity-100': isLoaded() }}
          onLoad={() => setIsLoaded(true)}
          onError={() => setImgSrc('/icons/app-logo/user.svg')}
        />
        <div
          class="absolute inset-0 w-full h-full bg-dark-500 flex items-center justify-center transition-opacity duration-300 pointer-events-none"
          classList={{ 'opacity-100': !isLoaded(), 'opacity-0': isLoaded() }}
        >
          <Icon src="/icons/app-logo/user.svg" class="w-5 h-5 opacity-50" />
        </div>
      </div>
      
      <div 
        class="absolute top-full left-1/2 -translate-x-1/2 mt-3 bg-dark min-w-[140px] border border-pri rounded-lg shadow-[0_4px_15px_rgba(0,0,0,0.4)] z-[1000] transition-all duration-200 p-1.5 before:content-[''] before:absolute before:-top-1.5 before:left-1/2 before:-translate-x-1/2 before:rotate-45 before:w-2.5 before:h-2.5 before:bg-dark before:border-l before:border-t before:border-pri"
        classList={{ 'invisible opacity-0': !isVisible(), 'visible opacity-100': isVisible() }}
      >
        <div class="user-menu-item" onClick={props.onEditAvatar}>
          <Icon src="/icons/app-logo/camera.svg" class="w-4 h-4" />
          更换头像
        </div>

        <Show
          when={props.isLoggedIn}
          fallback={
            <div class="user-menu-item" onClick={props.onLoginClick}>
              <Icon src="/icons/app-logo/user-profile.svg" class="w-4 h-4" />
              登录账号
            </div>
          }
        >
          <div class="h-[1px] bg-dark-300 my-1.5 mx-2 opacity-60"></div>
          <div class="user-menu-item font-medium">
            <Icon src="/icons/app-logo/info-circle.svg" class="w-4 h-4" />
            账号信息
          </div>
          <div class="user-menu-item" onClick={props.onLoginClick}>
            <Icon src="/icons/app-logo/switch-arrows.svg" class="w-4 h-4" />
            切换账号
          </div>
          <div 
            class="flex items-center gap-2.5 p-2.5 text-[#E08090] opacity-90 text-[13px] rounded-md transition-all hover:bg-[rgba(255,77,79,0.15)] hover:text-[#E08090]" 
            onClick={props.onLogout}
          >
            <Icon src="/icons/app-logo/logout.svg" class="w-4 h-4" />
            退出登录
          </div>
        </Show>
      </div>
    </div>
  );
};

export default UserDropdown;