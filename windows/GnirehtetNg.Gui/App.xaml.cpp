#include "pch.h"
#include "App.xaml.h"
#include "MainWindow.xaml.h"

#include <microsoft.ui.xaml.window.h>

#include <filesystem>
#include <fstream>

namespace
{
    void RecordStartupFailure(wchar_t const* stage, winrt::hresult_error const& failure) noexcept
    {
        try
        {
            std::filesystem::path path = std::filesystem::temp_directory_path() /
                L"gnirehtet-ng-gui-startup-error.txt";
            std::wofstream stream(path, std::ios::trunc);
            stream << stage << L" failed with HRESULT 0x" << std::hex
                   << static_cast<uint32_t>(failure.code()) << L": " << failure.message().c_str();
        }
        catch (...)
        {
        }
    }
}

namespace winrt::GnirehtetNg::Gui::implementation
{
    App::App()
    {
        try
        {
            InitializeComponent();
        }
        catch (winrt::hresult_error const& failure)
        {
            RecordStartupFailure(L"App.InitializeComponent", failure);
            throw;
        }
    }

    void App::OnLaunched(Microsoft::UI::Xaml::LaunchActivatedEventArgs const&)
    {
        try
        {
            auto mainWindow = winrt::make<MainWindow>();
            window_ = mainWindow;
            window_.Activate();
            HWND windowHandle{};
            winrt::check_hresult(mainWindow.as<::IWindowNative>()->get_WindowHandle(&windowHandle));
            SetWindowPos(windowHandle, nullptr, 0, 0, 1220, 820, SWP_NOMOVE | SWP_NOZORDER);
            winrt::get_self<MainWindow>(mainWindow)->InitializeNativeWindow(windowHandle);
        }
        catch (winrt::hresult_error const& failure)
        {
            RecordStartupFailure(L"MainWindow construction", failure);
            throw;
        }
    }
}
