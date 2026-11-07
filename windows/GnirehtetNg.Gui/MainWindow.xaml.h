#pragma once

#include "MainWindow.g.h"

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

#include <commctrl.h>

namespace winrt::GnirehtetNg::Gui::implementation
{
    struct MainWindow : MainWindowT<MainWindow>
    {
        MainWindow();
        ~MainWindow();
        void InitializeNativeWindow(HWND windowHandle);

        void Navigation_SelectionChanged(
            Microsoft::UI::Xaml::Controls::NavigationView const&,
            Microsoft::UI::Xaml::Controls::NavigationViewSelectionChangedEventArgs const& args);
        void RefreshButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void RefreshDevicesButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void StartButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void StopButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void RestartButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void OpenLogsButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void LogFilter_Changed(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::Controls::SelectionChangedEventArgs const&);
        void LogTextFilter_Changed(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::Controls::TextChangedEventArgs const&);
        void ClearLogsButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void CopySelectedLogsButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void CopyAllLogsButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void ExportLogsButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void ExportDiagnosticsButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void SaveSettingsButton_Click(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);
        void AutoReconnectToggle_Toggled(
            Windows::Foundation::IInspectable const&,
            Microsoft::UI::Xaml::RoutedEventArgs const&);

    private:
        struct LogRecord
        {
            uint64_t sequence{};
            winrt::hstring level;
            winrt::hstring component;
            winrt::hstring device;
            winrt::hstring display;
            std::wstring searchable;
            std::wstring device_lower;
        };

        Microsoft::UI::Xaml::DispatcherTimer refreshTimer_{nullptr};
        std::vector<LogRecord> logs_;
        uint64_t lastLogSequence_{};
        uint32_t currentCorePid_{};
        winrt::hstring currentSessionState_{L"unknown"};
        winrt::hstring currentPage_{L"dashboard"};
        winrt::hstring lastDeviceSignature_;
        winrt::hstring lastDevicesInfoMessage_;
        bool pointerOverLogList_{};
        Microsoft::UI::Xaml::DispatcherTimer searchDebounceTimer_{nullptr};
        unsigned refreshTick_{};
        bool refreshInFlight_{};
        bool commandInFlight_{};
        bool devicesLoaded_{};
        bool hasVisibleDevice_{};
        bool hasReadyDevice_{};
        bool waitingForDevice_{};
        bool suppressAutoReconnectPersistence_{true};
        winrt::hstring pendingDeviceCommand_;
        HWND windowHandle_{};
        NOTIFYICONDATAW trayIcon_{};
        bool trayIconAdded_{};
        bool exiting_{};

        winrt::fire_and_forget RefreshAsync(bool includeDevices);
        winrt::fire_and_forget ExecuteSessionCommandAsync(winrt::hstring command);
        winrt::fire_and_forget ExportDiagnosticsAsync();
        void ApplySnapshot(Windows::Data::Json::JsonObject const& snapshot);
        void ApplyDevices(Windows::Data::Json::JsonArray const& devices);
        void ApplyLogs(Windows::Data::Json::JsonArray const& logs);
        void RenderLogs();
        void RenderAdbStates(winrt::hstring const& adbState);
        bool PassesLogFilter(
            LogRecord const& record,
            std::wstring const& search,
            std::wstring const& device,
            winrt::hstring const& level,
            winrt::hstring const& component);
        static int LogLevelRank(winrt::hstring const& value);
        double ValidatedPort();
        void WriteSessionSettingsFields(Windows::Data::Json::JsonObject const& payload);
        void SetPage(winrt::hstring const& tag);
        void SetBusy(bool busy);
        void ShowError(winrt::hstring const& context, winrt::hstring const& error);
        void ExecuteOrWaitForDevice(winrt::hstring const& command);
        void UpdateStandbyPresentation();
        Windows::Data::Json::JsonObject SessionPayload();
        Windows::Data::Json::JsonObject LogRequestPayload();
        void LoadSettings();
        void SaveSettings();
        std::filesystem::path UiStateDirectory();
        std::wstring JoinedVisibleLogs(bool selectedOnly);
        void AddTrayIcon();
        void RemoveTrayIcon() noexcept;
        void UpdateTrayTooltip(winrt::hstring const& session, winrt::hstring const& device);
        void ShowTrayMenu();
        void HandleTrayCommand(unsigned command);
        void ShowMainWindow();
        static LRESULT CALLBACK WindowSubclass(
            HWND window,
            UINT message,
            WPARAM wParam,
            LPARAM lParam,
            UINT_PTR subclassId,
            DWORD_PTR referenceData);
    };
}

namespace winrt::GnirehtetNg::Gui::factory_implementation
{
    struct MainWindow : MainWindowT<MainWindow, implementation::MainWindow>
    {
    };
}
