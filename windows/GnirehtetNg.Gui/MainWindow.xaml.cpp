#include "pch.h"
#include "MainWindow.xaml.h"
#include "IpcClient.h"
#include "Encoding.h"



#if __has_include("MainWindow.g.cpp")
#include "MainWindow.g.cpp"
#endif



#include <winrt/Microsoft.UI.Windowing.h>
#include <microsoft.ui.xaml.window.h>
#include <ShlObj.h>



#include <algorithm>
#include <chrono>
#include <cmath>
#include <cwctype>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iterator>
#include <limits>
#include <map>
#include <sstream>



#pragma comment(lib, "Shell32.lib")
#pragma comment(lib, "Comctl32.lib")



namespace
{
    using namespace winrt;
    using namespace Windows::Data::Json;
    using namespace Microsoft::UI::Xaml;
    using namespace Microsoft::UI::Xaml::Controls;
    using namespace Microsoft::UI::Xaml::Media;
    using GnirehtetNgEncoding::Utf8ToWide;
    using GnirehtetNgEncoding::WideToUtf8;



    hstring JsonString(JsonObject const& object, wchar_t const* name, wchar_t const* fallback = L"")
    {
        auto const value = object.GetNamedValue(name, nullptr);
        return value && value.ValueType() == JsonValueType::String ? value.GetString() : hstring(fallback);
    }



    uint64_t JsonUnsigned(JsonObject const& object, wchar_t const* name)
    {
        auto const jsonValue = object.GetNamedValue(name, nullptr);
        if (!jsonValue || jsonValue.ValueType() != JsonValueType::Number)
        {
            return 0;
        }
        double const value = jsonValue.GetNumber();
        if (!std::isfinite(value) || value <= 0.0)
        {
            return 0;
        }
        double const maximum = static_cast<double>(std::numeric_limits<uint64_t>::max());
        return static_cast<uint64_t>(std::min(value, maximum));
    }

    uint64_t FirstSequence(IJsonValue const& events)
    {
        if (!events || events.ValueType() != JsonValueType::Array)
        {
            return 0;
        }
        JsonArray const array = events.GetArray();
        for (IJsonValue const& value : array)
        {
            if (value.ValueType() == JsonValueType::Object)
            {
                return JsonUnsigned(value.GetObject(), L"sequence");
            }
        }
        return 0;
    }



    std::wstring Lower(std::wstring value)
    {
        std::transform(value.begin(), value.end(), value.begin(), [](wchar_t character) {
            return static_cast<wchar_t>(std::towlower(character));
        });
        return value;
    }



    hstring StatusLabel(hstring const& state)
    {
        std::wstring label = state.c_str();
        std::transform(label.begin(), label.end(), label.begin(), [](wchar_t character) {
            return character == L'_' ? L' ' : static_cast<wchar_t>(std::towupper(character));
        });
        return hstring(label);
    }



    std::wstring Pad(std::wstring value, size_t length)
    {
        if (value.size() < length)
        {
            value.append(length - value.size(), L' ');
        }
        return value;
    }



    SolidColorBrush MakeBrush(uint8_t red, uint8_t green, uint8_t blue)
    {
        static std::map<uint32_t, SolidColorBrush> brushes;
        uint32_t const key =
            (static_cast<uint32_t>(red) << 16) | (static_cast<uint32_t>(green) << 8) | blue;
        auto const found = brushes.find(key);
        if (found != brushes.end())
        {
            return found->second;
        }
        SolidColorBrush brush(Windows::UI::Color{255, red, green, blue});
        brushes.emplace(key, brush);
        return brush;
    }

    bool SameColor(Brush const& existing, SolidColorBrush const& replacement)
    {
        auto const current = existing.try_as<SolidColorBrush>();
        if (!current)
        {
            return false;
        }
        Windows::UI::Color const left = current.Color();
        Windows::UI::Color const right = replacement.Color();
        return left.R == right.R && left.G == right.G && left.B == right.B && left.A == right.A;
    }

    void SetTextIfChanged(TextBlock const& control, hstring const& value)
    {
        if (control.Text() != value)
        {
            control.Text(value);
        }
    }

    void SetForegroundIfChanged(TextBlock const& control, SolidColorBrush const& brush)
    {
        if (!SameColor(control.Foreground(), brush))
        {
            control.Foreground(brush);
        }
    }

    void SetBackgroundIfChanged(Border const& control, SolidColorBrush const& brush)
    {
        if (!SameColor(control.Background(), brush))
        {
            control.Background(brush);
        }
    }

    void SetHealthText(TextBlock const& control, hstring const& state)
    {
        hstring label = L"Unknown";
        SolidColorBrush brush = MakeBrush(125, 133, 144);
        if (state == L"ready")
        {
            label = L"Ready";
            brush = MakeBrush(52, 199, 89);
        }
        else if (state == L"degraded")
        {
            label = L"Degraded";
            brush = MakeBrush(242, 181, 49);
        }
        else if (state == L"unavailable")
        {
            label = L"Unavailable";
            brush = MakeBrush(245, 91, 91);
        }
        SetTextIfChanged(control, hstring(std::wstring(L"● ") + label.c_str()));
        SetForegroundIfChanged(control, brush);
    }



    hstring FormatBytes(uint64_t bytes, bool perSecond)
    {
        wchar_t const* units[] = {L"B", L"KiB", L"MiB", L"GiB", L"TiB"};
        double value = static_cast<double>(bytes);
        size_t unit = 0;
        while (value >= 1024.0 && unit + 1 < std::size(units))
        {
            value /= 1024.0;
            ++unit;
        }
        std::wstringstream output;
        if (unit == 0)
        {
            output << static_cast<uint64_t>(value);
        }
        else
        {
            output << std::fixed << std::setprecision(value >= 100.0 ? 0 : 1) << value;
        }
        output << L' ' << units[unit];
        if (perSecond)
        {
            output << L"/s";
        }
        return hstring(output.str());
    }



    hstring FormatUptime(uint64_t startedUnixMs)
    {
        if (startedUnixMs == 0)
        {
            return L"--";
        }
        uint64_t const nowMs = static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::system_clock::now().time_since_epoch()).count());
        uint64_t seconds = nowMs > startedUnixMs ? (nowMs - startedUnixMs) / 1000 : 0;
        uint64_t const hours = seconds / 3600;
        seconds %= 3600;
        uint64_t const minutes = seconds / 60;
        seconds %= 60;
        std::wstringstream output;
        if (hours != 0)
        {
            output << hours << L'h' << L' ';
        }
        output << minutes << L'm' << L' ' << seconds << L's';
        return hstring(output.str());
    }



    hstring SelectedComboText(ComboBox const& combo)
    {
        auto const item = combo.SelectedItem().try_as<ComboBoxItem>();
        return item ? unbox_value_or<hstring>(item.Content(), hstring{}) : hstring{};
    }



    std::wstring TimestampForFilename()
    {
        SYSTEMTIME value{};
        GetLocalTime(&value);
        std::wstringstream result;
        result << std::setfill(L'0') << std::setw(4) << value.wYear << std::setw(2) << value.wMonth
               << std::setw(2) << value.wDay << L'-' << std::setw(2) << value.wHour << std::setw(2)
               << value.wMinute << std::setw(2) << value.wSecond;
        return result.str();
    }



    void RecordRuntimeFailure(wchar_t const* stage, hresult_error const& failure) noexcept
    {
        try
        {
            std::filesystem::path const path = std::filesystem::temp_directory_path() /
                L"gnirehtet-ng-gui-runtime-error.txt";
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
    namespace
    {
        constexpr UINT TrayCallbackMessage = WM_APP + 42;
        constexpr UINT_PTR WindowSubclassId = 1;
        constexpr unsigned TrayOpen = 41001;
        constexpr unsigned TrayStart = 41002;
        constexpr unsigned TrayStop = 41003;
        constexpr unsigned TrayRestart = 41004;
        constexpr unsigned TrayAuto = 41005;
        constexpr unsigned TrayExit = 41006;
    }



    MainWindow::MainWindow()
    {
        InitializeComponent();
        Title(L"Gnirehtet NG");
        try
        {
            SystemBackdrop(MicaBackdrop{});
        }
        catch (...)
        {
            // Mica is optional on unsupported or remote-rendered Windows configurations.
        }



        LoadSettings();
        suppressAutoReconnectPersistence_ = false;
        refreshTimer_ = DispatcherTimer{};
        refreshTimer_.Interval(std::chrono::seconds(1));
        refreshTimer_.Tick([weak = get_weak()](IInspectable const&, IInspectable const&) {
            if (auto self = weak.get())
            {
                self->RefreshAsync(false);
            }
        });
        LogList().PointerEntered(
            [weak = get_weak()](
                IInspectable const&, Microsoft::UI::Xaml::Input::PointerRoutedEventArgs const&) {
                if (auto self = weak.get())
                {
                    self->pointerOverLogList_ = true;
                }
            });
        LogList().PointerExited(
            [weak = get_weak()](
                IInspectable const&, Microsoft::UI::Xaml::Input::PointerRoutedEventArgs const&) {
                if (auto self = weak.get())
                {
                    self->pointerOverLogList_ = false;
                }
            });
        RootGrid().Loaded([weak = get_weak()](IInspectable const&, RoutedEventArgs const&) {
            if (auto self = weak.get())
            {
                self->refreshTimer_.Start();
                self->RefreshAsync(true);
            }
        });

        // Taskbar/window icon: same flat mark as the embedded executable icon
        try
        {
            std::wstring module(32'768, L'\0');
            DWORD const moduleLength =
                GetModuleFileNameW(nullptr, module.data(), static_cast<DWORD>(module.size()));
            if (moduleLength > 0 && moduleLength < module.size())
            {
                module.resize(moduleLength);
                std::filesystem::path const iconPath =
                    std::filesystem::path(module).parent_path() / L"app-icon.ico";
                if (std::filesystem::is_regular_file(iconPath))
                {
                    AppWindow().SetIcon(iconPath.c_str());
                }
            }
        }
        catch (...)
        {
            // icon setup must never break startup
        }
    }



    MainWindow::~MainWindow()
    {
        RemoveTrayIcon();
        if (windowHandle_)
        {
            RemoveWindowSubclass(windowHandle_, WindowSubclass, WindowSubclassId);
        }
    }



    void MainWindow::InitializeNativeWindow(HWND windowHandle)
    {
        windowHandle_ = windowHandle;
        if (!SetWindowSubclass(
                windowHandle_, WindowSubclass, WindowSubclassId, reinterpret_cast<DWORD_PTR>(this)))
        {
            throw hresult_error(HRESULT_FROM_WIN32(GetLastError()), L"Could not subclass the WinUI window");
        }
        AddTrayIcon();
        if (!trayIconAdded_)
        {
            MinimizeToTrayToggle().IsOn(false);
            SettingsStatus().Text(L"The system tray is unavailable; closing the window will exit the GUI.");
        }
        Closed([weak = get_weak()](IInspectable const&, WindowEventArgs const& args) {
            if (auto self = weak.get())
            {
                if (!self->exiting_ && self->MinimizeToTrayToggle().IsOn())
                {
                    args.Handled(true);
                    ShowWindow(self->windowHandle_, SW_HIDE);
                }
                else
                {
                    self->RemoveTrayIcon();
                }
            }
        });
    }



    void MainWindow::AddTrayIcon()
    {
        if (trayIconAdded_ || !windowHandle_)
        {
            return;
        }
        trayIcon_ = {};
        trayIcon_.cbSize = sizeof(trayIcon_);
        trayIcon_.hWnd = windowHandle_;
        trayIcon_.uID = 1;
        trayIcon_.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        trayIcon_.uCallbackMessage = TrayCallbackMessage;
        trayIcon_.hIcon = LoadIconW(GetModuleHandleW(nullptr), MAKEINTRESOURCEW(1));
        wcscpy_s(trayIcon_.szTip, L"Gnirehtet NG - Controller starting");
        if (!Shell_NotifyIconW(NIM_ADD, &trayIcon_))
        {
            DWORD const error = GetLastError();
            try
            {
                std::wofstream stream(
                    std::filesystem::temp_directory_path() / L"gnirehtet-ng-gui-tray-error.txt",
                    std::ios::trunc);
                stream << L"Shell_NotifyIconW(NIM_ADD) failed with Win32 error " << error
                       << L"; hwnd=" << windowHandle_ << L"; icon=" << trayIcon_.hIcon;
            }
            catch (...)
            {
            }
            trayIcon_ = {};
            return;
        }
        trayIconAdded_ = true;
    }



    void MainWindow::RemoveTrayIcon() noexcept
    {
        if (trayIconAdded_)
        {
            Shell_NotifyIconW(NIM_DELETE, &trayIcon_);
            trayIconAdded_ = false;
        }
    }



    void MainWindow::UpdateTrayTooltip(hstring const& session, hstring const& device)
    {
        if (!trayIconAdded_)
        {
            return;
        }
        std::wstring tooltip = L"Gnirehtet NG - " + std::wstring(session);
        if (!device.empty())
        {
            tooltip += L" - " + std::wstring(device);
        }
        if (tooltip.size() >= std::size(trayIcon_.szTip))
        {
            tooltip.resize(std::size(trayIcon_.szTip) - 1);
        }
        wcscpy_s(trayIcon_.szTip, tooltip.c_str());
        trayIcon_.uFlags = NIF_TIP;
        Shell_NotifyIconW(NIM_MODIFY, &trayIcon_);
    }



    void MainWindow::ShowMainWindow()
    {
        ShowWindow(windowHandle_, SW_SHOW);
        ShowWindow(windowHandle_, SW_RESTORE);
        SetForegroundWindow(windowHandle_);
        Activate();
    }



    void MainWindow::ShowTrayMenu()
    {
        HMENU menu = CreatePopupMenu();
        if (!menu)
        {
            return;
        }
        AppendMenuW(menu, MF_STRING, TrayOpen, L"Open Gnirehtet NG");
        AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
        AppendMenuW(menu, MF_STRING, TrayStart, L"Start");
        AppendMenuW(menu, MF_STRING, TrayStop, L"Stop");
        AppendMenuW(menu, MF_STRING, TrayRestart, L"Restart");
        AppendMenuW(
            menu,
            MF_STRING | (AutoReconnectToggle().IsOn() ? MF_CHECKED : MF_UNCHECKED),
            TrayAuto,
            L"Auto reconnect");
        AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
        AppendMenuW(menu, MF_STRING, TrayExit, L"Exit");
        POINT cursor{};
        GetCursorPos(&cursor);
        SetForegroundWindow(windowHandle_);
        unsigned const command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            cursor.x,
            cursor.y,
            0,
            windowHandle_,
            nullptr);
        DestroyMenu(menu);
        if (command != 0)
        {
            HandleTrayCommand(command);
        }
    }



    void MainWindow::HandleTrayCommand(unsigned command)
    {
        switch (command)
        {
        case TrayOpen:
            ShowMainWindow();
            break;
        case TrayStart:
            ExecuteOrWaitForDevice(L"start");
            break;
        case TrayStop:
            waitingForDevice_ = false;
            pendingDeviceCommand_.clear();
            ExecuteSessionCommandAsync(L"stop");
            break;
        case TrayRestart:
            ExecuteOrWaitForDevice(L"restart");
            break;
        case TrayAuto:
            AutoReconnectToggle().IsOn(!AutoReconnectToggle().IsOn());
            break;
        case TrayExit:
            exiting_ = true;
            RemoveTrayIcon();
            Close();
            break;
        default:
            break;
        }
    }



    LRESULT CALLBACK MainWindow::WindowSubclass(
        HWND window,
        UINT message,
        WPARAM wParam,
        LPARAM lParam,
        UINT_PTR subclassId,
        DWORD_PTR referenceData)
    {
        auto* self = reinterpret_cast<MainWindow*>(referenceData);
        if (message == WM_CLOSE && self && !self->exiting_ &&
            self->MinimizeToTrayToggle().IsOn())
        {
            ShowWindow(window, SW_HIDE);
            return 0;
        }
        if (message == TrayCallbackMessage && self)
        {
            if (lParam == WM_LBUTTONUP || lParam == WM_LBUTTONDBLCLK)
            {
                self->ShowMainWindow();
            }
            else if (lParam == WM_RBUTTONUP || lParam == WM_CONTEXTMENU)
            {
                self->ShowTrayMenu();
            }
            return 0;
        }
        if (message == WM_NCDESTROY)
        {
            RemoveWindowSubclass(window, WindowSubclass, subclassId);
        }
        return DefSubclassProc(window, message, wParam, lParam);
    }



    void MainWindow::Navigation_SelectionChanged(
        NavigationView const&,
        NavigationViewSelectionChangedEventArgs const& args)
    {
        auto const item = args.SelectedItemContainer();
        if (item)
        {
            SetPage(unbox_value_or<hstring>(item.Tag(), hstring{L"dashboard"}));
        }
    }



    void MainWindow::SetPage(hstring const& tag)
    {
        currentPage_ = tag;
        DashboardPage().Visibility(tag == L"dashboard" ? Visibility::Visible : Visibility::Collapsed);
        DevicesPage().Visibility(tag == L"devices" ? Visibility::Visible : Visibility::Collapsed);
        LogsPage().Visibility(tag == L"logs" ? Visibility::Visible : Visibility::Collapsed);
        DiagnosticsPage().Visibility(tag == L"diagnostics" ? Visibility::Visible : Visibility::Collapsed);
        SettingsPage().Visibility(tag == L"settings" ? Visibility::Visible : Visibility::Collapsed);
        AboutPage().Visibility(tag == L"about" ? Visibility::Visible : Visibility::Collapsed);
        if (tag == L"devices")
        {
            RefreshAsync(true);
        }
        else if (tag == L"logs")
        {
            RenderLogs();
        }
    }



    void MainWindow::RefreshButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        RefreshAsync(true);
    }



    void MainWindow::RefreshDevicesButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        RefreshAsync(true);
    }



    void MainWindow::StartButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        ExecuteOrWaitForDevice(L"start");
    }



    void MainWindow::StopButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        waitingForDevice_ = false;
        pendingDeviceCommand_.clear();
        ExecuteSessionCommandAsync(L"stop");
    }



    void MainWindow::RestartButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        ExecuteOrWaitForDevice(L"restart");
    }



    void MainWindow::ExecuteOrWaitForDevice(hstring const& command)
    {
        if (devicesLoaded_ && !hasReadyDevice_)
        {
            if (AutoReconnectToggle().IsOn())
            {
                waitingForDevice_ = true;
                pendingDeviceCommand_ = command;
                UpdateStandbyPresentation();
                RefreshAsync(true);
            }
            else
            {
                ShowError(L"No authorized device", L"Connect and authorize an Android device, then try again.");
            }
            return;
        }
        waitingForDevice_ = false;
        pendingDeviceCommand_.clear();
        ExecuteSessionCommandAsync(command);
    }



    void MainWindow::AutoReconnectToggle_Toggled(IInspectable const&, RoutedEventArgs const&)
    {
        if (suppressAutoReconnectPersistence_)
        {
            return;
        }
        if (!AutoReconnectToggle().IsOn())
        {
            waitingForDevice_ = false;
            pendingDeviceCommand_.clear();
        }
        try
        {
            SaveSettings();
        }
        catch (std::exception const& failure)
        {
            SettingsStatus().Text(to_hstring(std::string("Could not save automatic recovery setting: ") + failure.what()));
        }
        UpdateStandbyPresentation();
    }



    void MainWindow::OpenLogsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        for (auto const& menuItem : Navigation().MenuItems())
        {
            auto const item = menuItem.try_as<NavigationViewItem>();
            if (item && unbox_value_or<hstring>(item.Tag(), hstring{}) == L"logs")
            {
                Navigation().SelectedItem(item);
                SetPage(L"logs");
                break;
            }
        }
    }



    double MainWindow::ValidatedPort()
    {
        double const port = PortNumber().Value();
        return std::isfinite(port) && port >= 1024.0 && port <= 65535.0 ? port : 31416.0;
    }



    void MainWindow::WriteSessionSettingsFields(JsonObject const& payload)
    {
        payload.SetNamedValue(L"port", JsonValue::CreateNumberValue(ValidatedPort()));
        hstring const dns = DnsText().Text();
        if (!dns.empty())
        {
            payload.SetNamedValue(L"dns_servers", JsonValue::CreateStringValue(dns));
        }
        hstring const routes = RoutesText().Text();
        if (!routes.empty())
        {
            payload.SetNamedValue(L"routes", JsonValue::CreateStringValue(routes));
        }
        payload.SetNamedValue(
            L"auto_reconnect", JsonValue::CreateBooleanValue(AutoReconnectToggle().IsOn()));
    }



    JsonObject MainWindow::SessionPayload()
    {
        JsonObject payload;
        auto const item = SelectedDevice().SelectedItem().try_as<ComboBoxItem>();
        if (item)
        {
            hstring const serial = unbox_value_or<hstring>(item.Tag(), hstring{});
            if (!serial.empty())
            {
                payload.SetNamedValue(L"serial", JsonValue::CreateStringValue(serial));
            }
        }
        WriteSessionSettingsFields(payload);
        return payload;
    }



    JsonObject MainWindow::LogRequestPayload()
    {
        JsonObject payload;
        payload.SetNamedValue(L"limit", JsonValue::CreateNumberValue(1000));
        if (lastLogSequence_ != 0)
        {
            payload.SetNamedValue(
                L"after_sequence",
                JsonValue::CreateNumberValue(static_cast<double>(lastLogSequence_)));
        }
        return payload;
    }



    fire_and_forget MainWindow::RefreshAsync(bool includeDevices)
    {
        auto lifetime = get_strong();
        if (refreshInFlight_)
        {
            co_return;
        }
        refreshInFlight_ = true;
        ++refreshTick_;
        // Device discovery stays active in standby without repeatedly starting a session.
        includeDevices = includeDevices || refreshTick_ % 5 == 0;
        bool const includeLogs = !PauseLogsToggle().IsOn();
        JsonObject const logPayload = includeLogs ? LogRequestPayload() : JsonObject{};
        uint64_t const cursorBefore = lastLogSequence_;
        auto const queue = DispatcherQueue();
        IJsonValue snapshotValue{nullptr};
        IJsonValue devicesValue{nullptr};
        IJsonValue logsValue{nullptr};
        hstring error;

        co_await resume_background();
        try
        {
            snapshotValue = gnirehtet::ipc::SendRequest(L"get_status", JsonObject{});
            if (includeDevices)
            {
                devicesValue = gnirehtet::ipc::SendRequest(L"get_devices", JsonObject{});
            }
            if (includeLogs)
            {
                logsValue = gnirehtet::ipc::SendRequest(L"get_logs", logPayload);
                // A 64 KiB-bounded response can trim the oldest events of a batch; if the
                // delivered batch starts after a gap, page backward to fill it. Bound the
                // per-tick workload: remaining gap events are filled on later ticks.
                constexpr unsigned MaxBackwardPages = 4;
                constexpr uint32_t MaximumPerTickLogEvents = 1'000;
                uint64_t first = FirstSequence(logsValue);
                for (unsigned page = 0;
                     page < MaxBackwardPages && cursorBefore != 0 && first != 0 &&
                     first > cursorBefore + 1;
                     ++page)
                {
                    JsonObject older;
                    older.SetNamedValue(L"limit", JsonValue::CreateNumberValue(1000));
                    older.SetNamedValue(
                        L"before_sequence",
                        JsonValue::CreateNumberValue(static_cast<double>(first)));
                    IJsonValue const olderValue =
                        gnirehtet::ipc::SendRequest(L"get_logs", older);
                    uint64_t const olderFirst = FirstSequence(olderValue);
                    if (olderFirst == 0)
                    {
                        break;
                    }
                    JsonArray combined;
                    uint32_t combinedCount = 0;
                    for (IJsonValue const& value : olderValue.GetArray())
                    {
                        if (combinedCount >= MaximumPerTickLogEvents)
                        {
                            break;
                        }
                        combined.Append(value);
                        ++combinedCount;
                    }
                    for (IJsonValue const& value : logsValue.GetArray())
                    {
                        if (combinedCount >= MaximumPerTickLogEvents)
                        {
                            break;
                        }
                        combined.Append(value);
                        ++combinedCount;
                    }
                    logsValue = combined;
                    first = olderFirst;
                }
            }
        }
        catch (gnirehtet::ipc::IpcError const& failure)
        {
            error = to_hstring(failure.code() + ": " + failure.what());
        }
        catch (std::exception const& failure)
        {
            error = to_hstring(failure.what());
        }



        queue.TryEnqueue([this, lifetime, snapshotValue, devicesValue, logsValue, error]() {
            try
            {
                refreshInFlight_ = false;
                if (!error.empty())
                {
                    ShowError(L"Controller unavailable", error);
                    return;
                }
                if (snapshotValue && snapshotValue.ValueType() == JsonValueType::Object)
                {
                    ApplySnapshot(snapshotValue.GetObject());
                }
                if (devicesValue && devicesValue.ValueType() == JsonValueType::Array)
                {
                    ApplyDevices(devicesValue.GetArray());
                }
                if (logsValue && logsValue.ValueType() == JsonValueType::Array)
                {
                    ApplyLogs(logsValue.GetArray());
                }
            }
            catch (hresult_error const& failure)
            {
                RecordRuntimeFailure(L"Refresh UI dispatch", failure);
                refreshInFlight_ = false;
                ShowError(L"Could not render controller state", failure.message());
            }
        });
    }



    fire_and_forget MainWindow::ExecuteSessionCommandAsync(hstring command)
    {
        auto lifetime = get_strong();
        if (commandInFlight_)
        {
            co_return;
        }
        commandInFlight_ = true;
        SetBusy(true);
        JsonObject const request = command == L"stop" ? JsonObject{} : SessionPayload();
        auto const queue = DispatcherQueue();
        IJsonValue result{nullptr};
        hstring error;



        co_await resume_background();
        try
        {
            result = gnirehtet::ipc::SendRequest(command, request);
        }
        catch (gnirehtet::ipc::IpcError const& failure)
        {
            error = to_hstring(failure.code() + ": " + failure.what());
        }
        catch (std::exception const& failure)
        {
            error = to_hstring(failure.what());
        }



        queue.TryEnqueue([this, lifetime, result, error, command]() {
            commandInFlight_ = false;
            SetBusy(false);
            if (!error.empty())
            {
                bool const deviceMissing = std::wstring(error).rfind(L"usb_disconnected:", 0) == 0;
                if (deviceMissing && AutoReconnectToggle().IsOn() &&
                    (command == L"start" || command == L"restart"))
                {
                    waitingForDevice_ = true;
                    pendingDeviceCommand_ = command;
                    hasReadyDevice_ = false;
                    UpdateStandbyPresentation();
                    RefreshAsync(true);
                    return;
                }
                ShowError(command, error);
                return;
            }
            if (result && result.ValueType() == JsonValueType::Object)
            {
                ApplySnapshot(result.GetObject());
            }
            RefreshAsync(true);
        });
    }



    void MainWindow::SetBusy(bool busy)
    {
        StartButton().IsEnabled(!busy);
        StopButton().IsEnabled(!busy);
        RestartButton().IsEnabled(!busy);
        ExportDiagnosticsButton().IsEnabled(!busy);
    }



    void MainWindow::ShowError(hstring const& context, hstring const& error)
    {
        ControllerStatusText().Text(hstring(std::wstring(context) + L": " + error.c_str()));
        StatusBadgeText().Text(L"ATTENTION");
        StatusBadge().Background(MakeBrush(155, 47, 47));
        LastFault().Text(error);
    }



    void MainWindow::RenderAdbStates(hstring const& adbState)
    {
        if (adbState == L"ready")
        {
            SetTextIfChanged(UsbState(), L"● Connected");
            SetForegroundIfChanged(UsbState(), MakeBrush(52, 199, 89));
            SetTextIfChanged(AdbState(), L"● Ready");
            SetForegroundIfChanged(AdbState(), MakeBrush(52, 199, 89));
            SetTextIfChanged(AuthorizationState(), L"● Authorized");
            SetForegroundIfChanged(AuthorizationState(), MakeBrush(52, 199, 89));
        }
        else if (adbState == L"unauthorized")
        {
            SetTextIfChanged(UsbState(), L"● Connected");
            SetForegroundIfChanged(UsbState(), MakeBrush(52, 199, 89));
            SetTextIfChanged(AdbState(), L"● Detected");
            SetForegroundIfChanged(AdbState(), MakeBrush(242, 181, 49));
            SetTextIfChanged(AuthorizationState(), L"● Required");
            SetForegroundIfChanged(AuthorizationState(), MakeBrush(245, 91, 91));
        }
        else if (adbState == L"offline" || adbState == L"usb_found")
        {
            SetTextIfChanged(UsbState(), L"● Connected");
            SetForegroundIfChanged(UsbState(), MakeBrush(52, 199, 89));
            SetTextIfChanged(AdbState(), L"● Offline");
            SetForegroundIfChanged(AdbState(), MakeBrush(242, 181, 49));
            SetTextIfChanged(AuthorizationState(), L"● Unknown");
            SetForegroundIfChanged(AuthorizationState(), MakeBrush(125, 133, 144));
        }
        else
        {
            SetTextIfChanged(UsbState(), L"● Disconnected");
            SetForegroundIfChanged(UsbState(), MakeBrush(125, 133, 144));
            SetTextIfChanged(AdbState(), L"● Unavailable");
            SetForegroundIfChanged(AdbState(), MakeBrush(125, 133, 144));
            SetTextIfChanged(AuthorizationState(), L"● Unknown");
            SetForegroundIfChanged(AuthorizationState(), MakeBrush(125, 133, 144));
        }
    }



    void MainWindow::ApplySnapshot(JsonObject const& snapshot)
    {
        uint32_t const corePid = static_cast<uint32_t>(JsonUnsigned(snapshot, L"core_pid"));
        if (currentCorePid_ != 0 && currentCorePid_ != corePid)
        {
            logs_.clear();
            lastLogSequence_ = 0;
        }
        currentCorePid_ = corePid;



        hstring const session = JsonString(snapshot, L"session_state", L"unknown");
        currentSessionState_ = session;
        SetTextIfChanged(StatusBadgeText(), StatusLabel(session));
        SetTextIfChanged(
            RevisionText(),
            hstring(L"Revision " + std::to_wstring(JsonUnsigned(snapshot, L"revision"))));
        SetTextIfChanged(
            ControllerStatusText(),
            hstring(
                L"Local controller PID " + std::to_wstring(corePid) + L" · relay port " +
                std::to_wstring(JsonUnsigned(snapshot, L"relay_port"))));

        bool const online = session == L"internet_ready";
        bool const faulted = session == L"faulted" || session == L"vpn_permission_required";
        SetBackgroundIfChanged(
            StatusBadge(),
            online ? MakeBrush(30, 132, 73) : faulted ? MakeBrush(155, 47, 47) : MakeBrush(76, 92, 116));

        // The snapshot adb_state is only meaningful while a session exists (the health
        // checks maintain it); when idle or faulted the device list owns the ADB display
        // (ApplyDevices), so rendering the unknown snapshot state here would make the
        // dashboard flicker between the two sources.
        if (session != L"idle" && session != L"faulted")
        {
            RenderAdbStates(JsonString(snapshot, L"adb_state", L"unknown"));
        }

        hstring const apk = JsonString(snapshot, L"apk_state", L"unknown");
        SetTextIfChanged(
            ApkState(),
            apk == L"ready" ? L"● Ready" : apk == L"missing" ? L"● Missing" : L"● Unknown");
        SetForegroundIfChanged(
            ApkState(),
            apk == L"ready" ? MakeBrush(52, 199, 89) : apk == L"missing" ? MakeBrush(245, 91, 91) : MakeBrush(125, 133, 144));
        SetHealthText(TunnelState(), JsonString(snapshot, L"reverse_health", L"unknown"));
        SetHealthText(ProcessState(), JsonString(snapshot, L"process_health", L"unknown"));
        SetHealthText(VpnState(), JsonString(snapshot, L"vpn_health", L"unknown"));
        SetHealthText(RelayState(), JsonString(snapshot, L"relay_health", L"unknown"));
        SetHealthText(InternetState(), JsonString(snapshot, L"internet_health", L"unknown"));

        hstring selectedModel;
        auto const selectedValue = snapshot.GetNamedValue(L"selected_device", nullptr);
        if (selectedValue && selectedValue.ValueType() == JsonValueType::Object)
        {
            JsonObject const selected = selectedValue.GetObject();
            hstring const serial = JsonString(selected, L"serial", L"");
            hstring const model = JsonString(selected, L"model", L"Android device");
            selectedModel = model;
            SetTextIfChanged(
                DeviceDetails(), hstring(std::wstring(model) + L" · " + serial.c_str()));
        }
        else
        {
            SetTextIfChanged(DeviceDetails(), L"No device selected");
        }
        UpdateTrayTooltip(session, selectedModel);
        JsonObject const metrics = snapshot.GetNamedObject(L"metrics", JsonObject{});
        uint64_t const download = JsonUnsigned(metrics, L"bytes_to_device");
        uint64_t const upload = JsonUnsigned(metrics, L"bytes_from_device");
        SetTextIfChanged(DownloadRate(), FormatBytes(JsonUnsigned(metrics, L"bytes_to_device_per_second"), true));
        SetTextIfChanged(UploadRate(), FormatBytes(JsonUnsigned(metrics, L"bytes_from_device_per_second"), true));
        SetTextIfChanged(DownloadTotal(), hstring(std::wstring(FormatBytes(download, false)) + L" total"));
        SetTextIfChanged(UploadTotal(), hstring(std::wstring(FormatBytes(upload, false)) + L" total"));
        SetTextIfChanged(SessionUptime(), FormatUptime(JsonUnsigned(metrics, L"session_started_unix_ms")));
        SetTextIfChanged(ReconnectCount(), to_hstring(JsonUnsigned(metrics, L"reconnect_count")));

        auto const faultValue = snapshot.GetNamedValue(L"last_fault", nullptr);
        if (faultValue && faultValue.ValueType() == JsonValueType::Object)
        {
            JsonObject const fault = faultValue.GetObject();
            SetTextIfChanged(
                LastFault(),
                hstring(
                    std::wstring(JsonString(fault, L"code", L"fault")) + L": " +
                    JsonString(fault, L"message", L"Unknown controller fault").c_str()));
        }
        else
        {
            SetTextIfChanged(LastFault(), L"No recorded fault");
        }
        UpdateStandbyPresentation();
    }



    void MainWindow::UpdateStandbyPresentation()
    {
        bool const inactive = currentSessionState_ == L"idle" || currentSessionState_ == L"faulted";
        if (devicesLoaded_ && !hasVisibleDevice_ && inactive)
        {
            SetTextIfChanged(StatusBadgeText(), L"STANDBY");
            SetBackgroundIfChanged(StatusBadge(), MakeBrush(76, 92, 116));
            SetTextIfChanged(
                ControllerStatusText(),
                hstring(
                    std::wstring(L"STANDBY — waiting for an Android device · local controller PID ") +
                    std::to_wstring(currentCorePid_)));
        }

        hstring hint;
        if (waitingForDevice_)
        {
            hint = L"STANDBY — the requested connection will start once an authorized device appears.";
        }
        else if (!devicesLoaded_)
        {
            hint = L"Checking for an Android device…";
        }
        else if (!hasVisibleDevice_)
        {
            hint = L"STANDBY — device detection is active; no connection attempts are made.";
        }
        else if (!hasReadyDevice_)
        {
            hint = L"Device detected — authorize USB debugging to make it ready.";
        }
        else if (AutoReconnectToggle().IsOn())
        {
            hint = L"Automatic recovery is enabled for the active or next session.";
        }
        else
        {
            hint = L"Automatic recovery is off.";
        }
        SetTextIfChanged(AutoReconnectHint(), hint);
    }



    void MainWindow::ApplyDevices(JsonArray const& devices)
    {
        devicesLoaded_ = true;
        hasVisibleDevice_ = devices.Size() != 0;
        hasReadyDevice_ = false;
        std::wstring signature;
        for (IJsonValue const& value : devices)
        {
            if (value.ValueType() != JsonValueType::Object)
            {
                continue;
            }
            JsonObject const device = value.GetObject();
            hstring const serial = JsonString(device, L"serial", L"");
            hstring const state = JsonString(device, L"adb_state", L"");
            hasReadyDevice_ = hasReadyDevice_ || state == L"ready";
            signature += serial;
            signature += L'|';
            signature += state;
            signature += L'|';
            signature += JsonString(device, L"model", L"");
            signature += L'|';
            signature += JsonString(device, L"android_version", L"");
            signature += L';';
        }
        hstring const newSignature(signature);
        bool const devicesChanged = newSignature != lastDeviceSignature_;

        if (devicesChanged)
        {
            lastDeviceSignature_ = newSignature;
            hstring previousSerial;
            if (auto const previous = SelectedDevice().SelectedItem().try_as<ComboBoxItem>())
            {
                previousSerial = unbox_value_or<hstring>(previous.Tag(), hstring{});
            }
            SelectedDevice().Items().Clear();
            DeviceList().Items().Clear();
            for (IJsonValue const& value : devices)
            {
                if (value.ValueType() != JsonValueType::Object)
                {
                    continue;
                }
                JsonObject const device = value.GetObject();
                hstring const serial = JsonString(device, L"serial", L"unknown");
                hstring const model = JsonString(device, L"model", L"Android device");
                hstring const state = JsonString(device, L"adb_state", L"unknown");
                hstring const android = JsonString(device, L"android_version", L"");
                hstring const label = hstring(
                    std::wstring(model) +
                    (android.empty() ? L"" : L" · Android " + std::wstring(android)) +
                    L" · " + serial.c_str());
                ComboBoxItem item;
                item.Content(box_value(label));
                item.Tag(box_value(serial));
                SelectedDevice().Items().Append(item);
                DeviceList().Items().Append(box_value(hstring(
                    std::wstring(label) + L"\nADB: " + state.c_str() +
                    (android.empty() ? L"" : L"\nAndroid: " + std::wstring(android)))));
                if (serial == previousSerial)
                {
                    SelectedDevice().SelectedItem(item);
                }
            }
            if (SelectedDevice().SelectedIndex() < 0 && SelectedDevice().Items().Size() == 1)
            {
                SelectedDevice().SelectedIndex(0);
            }
        }
        if (currentSessionState_ == L"idle" || currentSessionState_ == L"faulted")
        {
            if (devices.Size() == 0)
            {
                RenderAdbStates(L"unknown");
            }
            else
            {
                RenderAdbStates(JsonString(devices.GetObjectAt(0), L"adb_state", L"unknown"));
            }
        }
        DevicesInfo().Severity(InfoBarSeverity::Informational);
        hstring const infoMessage = devices.Size() == 0
            ? L"No ADB device is currently visible. No phone action was attempted."
            : hstring(L"Discovered " + std::to_wstring(devices.Size()) + L" ADB device(s).");
        if (infoMessage != lastDevicesInfoMessage_)
        {
            lastDevicesInfoMessage_ = infoMessage;
            DevicesInfo().Message(infoMessage);
        }
        UpdateStandbyPresentation();

        if (waitingForDevice_ && hasReadyDevice_ && !commandInFlight_ && !pendingDeviceCommand_.empty())
        {
            hstring const command = pendingDeviceCommand_;
            waitingForDevice_ = false;
            pendingDeviceCommand_.clear();
            UpdateStandbyPresentation();
            ExecuteSessionCommandAsync(command);
        }
    }



    void MainWindow::ApplyLogs(JsonArray const& events)
    {
        std::vector<LogRecord> fresh;
        for (IJsonValue const& value : events)
        {
            if (value.ValueType() != JsonValueType::Object)
            {
                continue;
            }
            JsonObject const event = value.GetObject();
            uint64_t const sequence = JsonUnsigned(event, L"sequence");
            if (sequence <= lastLogSequence_)
            {
                continue;
            }
            LogRecord record;
            record.sequence = sequence;
            record.level = JsonString(event, L"level", L"INFO");
            record.component = JsonString(event, L"component", L"CORE");
            record.device = JsonString(event, L"device_id", L"");
            hstring timestamp = JsonString(event, L"timestamp", L"");
            std::wstring time = timestamp.c_str();
            if (time.size() >= 23)
            {
                time = time.substr(11, 12);
            }
            hstring const message = JsonString(event, L"message", L"");
            hstring const eventType = JsonString(event, L"event_type", L"");
            record.display = hstring(
                Pad(time, 12) + L" " + Pad(record.level.c_str(), 5) + L" " +
                Pad(record.component.c_str(), 10) + L" " + message.c_str() +
                (eventType.empty() ? L"" : L" [" + std::wstring(eventType) + L"]"));
            record.searchable = Lower(
                std::wstring(record.display) + L" " + record.device.c_str());
            record.device_lower = Lower(record.device.c_str());
            fresh.push_back(std::move(record));
            lastLogSequence_ = sequence;
        }
        if (fresh.empty())
        {
            return;
        }
        std::wstring const search = Lower(LogSearch().Text().c_str());
        std::wstring const device = Lower(DeviceFilter().Text().c_str());
        hstring const level = SelectedComboText(LogLevelFilter());
        hstring const component = SelectedComboText(ComponentFilter());
        for (LogRecord const& record : fresh)
        {
            if (PassesLogFilter(record, search, device, level, component))
            {
                LogList().Items().Append(box_value(record.display));
            }
        }
        constexpr size_t MaximumRenderedLogItems = 2'000;
        constexpr size_t RenderedLogTrimThreshold = 3'000;
        if (LogList().Items().Size() > RenderedLogTrimThreshold)
        {
            size_t const start = LogList().Items().Size() - MaximumRenderedLogItems;
            std::vector<IInspectable> tail;
            tail.reserve(MaximumRenderedLogItems);
            for (uint32_t index = static_cast<uint32_t>(start);
                 index < LogList().Items().Size();
                 ++index)
            {
                tail.push_back(LogList().Items().GetAt(index));
            }
            LogList().Items().ReplaceAll(tail);
        }
        LogCountText().Text(hstring(std::to_wstring(LogList().Items().Size()) + L" visible events"));
        if (AutoScrollToggle().IsOn() && currentPage_ == L"logs" && !pointerOverLogList_ &&
            LogList().Items().Size() != 0)
        {
            LogList().ScrollIntoView(LogList().Items().GetAt(LogList().Items().Size() - 1));
        }



        logs_.insert(
            logs_.end(), std::make_move_iterator(fresh.begin()), std::make_move_iterator(fresh.end()));
        constexpr size_t MaximumUiLogRecords = 50'000;
        if (logs_.size() > MaximumUiLogRecords)
        {
            logs_.erase(logs_.begin(), logs_.begin() + static_cast<ptrdiff_t>(logs_.size() - MaximumUiLogRecords));
        }



        DashboardLogList().Items().Clear();
        size_t const start = logs_.size() > 8 ? logs_.size() - 8 : 0;
        for (size_t index = start; index < logs_.size(); ++index)
        {
            DashboardLogList().Items().Append(box_value(logs_[index].display));
        }
    }



    int MainWindow::LogLevelRank(hstring const& value)
    {
        if (value == L"ERROR") return 5;
        if (value == L"WARN") return 4;
        if (value == L"INFO") return 3;
        if (value == L"DEBUG") return 2;
        return 1;
    }



    bool MainWindow::PassesLogFilter(
        LogRecord const& record,
        std::wstring const& search,
        std::wstring const& device,
        hstring const& level,
        hstring const& component)
    {
        if ((!level.empty() && level != L"ALL") &&
            LogLevelRank(record.level) < LogLevelRank(level))
        {
            return false;
        }
        if (!component.empty() && component != L"ALL" && record.component != component)
        {
            return false;
        }
        if (!device.empty() && record.device_lower.find(device) == std::wstring::npos)
        {
            return false;
        }
        if (!search.empty() && record.searchable.find(search) == std::wstring::npos)
        {
            return false;
        }
        return true;
    }



    void MainWindow::RenderLogs()
    {
        if (!LogList())
        {
            return;
        }
        constexpr size_t MaximumRenderedLogItems = 2'000;
        std::wstring const search = Lower(LogSearch().Text().c_str());
        std::wstring const device = Lower(DeviceFilter().Text().c_str());
        hstring const level = SelectedComboText(LogLevelFilter());
        hstring const component = SelectedComboText(ComponentFilter());
        std::vector<LogRecord const*> matches;
        for (LogRecord const& record : logs_)
        {
            if (PassesLogFilter(record, search, device, level, component))
            {
                matches.push_back(&record);
            }
        }
        size_t const skip = matches.size() > MaximumRenderedLogItems
            ? matches.size() - MaximumRenderedLogItems
            : 0;
        std::vector<IInspectable> rendered;
        rendered.reserve(matches.size() - skip);
        for (size_t index = skip; index < matches.size(); ++index)
        {
            rendered.push_back(box_value(matches[index]->display));
        }
        LogList().Items().ReplaceAll(rendered);
        LogCountText().Text(hstring(std::to_wstring(LogList().Items().Size()) + L" visible events"));
        if (AutoScrollToggle().IsOn() && !pointerOverLogList_ && LogList().Items().Size() != 0)
        {
            LogList().ScrollIntoView(LogList().Items().GetAt(LogList().Items().Size() - 1));
        }
    }



    void MainWindow::LogFilter_Changed(IInspectable const&, SelectionChangedEventArgs const&)
    {
        RenderLogs();
    }



    void MainWindow::LogTextFilter_Changed(IInspectable const&, TextChangedEventArgs const&)
    {
        if (!searchDebounceTimer_)
        {
            searchDebounceTimer_ = DispatcherTimer{};
            searchDebounceTimer_.Interval(std::chrono::milliseconds(200));
            searchDebounceTimer_.Tick([weak = get_weak()](IInspectable const&, IInspectable const&) {
                if (auto self = weak.get())
                {
                    self->searchDebounceTimer_.Stop();
                    self->RenderLogs();
                }
            });
        }
        searchDebounceTimer_.Stop();
        searchDebounceTimer_.Start();
    }



    void MainWindow::ClearLogsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        logs_.clear();
        LogList().Items().Clear();
        DashboardLogList().Items().Clear();
        LogCountText().Text(L"View cleared; controller logs remain available");
    }



    std::wstring MainWindow::JoinedVisibleLogs(bool selectedOnly)
    {
        std::wstringstream result;
        if (selectedOnly)
        {
            for (IInspectable const& item : LogList().SelectedItems())
            {
                result << unbox_value_or<hstring>(item, hstring{}).c_str() << L"\r\n";
            }
        }
        else
        {
            for (IInspectable const& item : LogList().Items())
            {
                result << unbox_value_or<hstring>(item, hstring{}).c_str() << L"\r\n";
            }
        }
        return result.str();
    }



    void MainWindow::CopySelectedLogsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        Windows::ApplicationModel::DataTransfer::DataPackage package;
        package.SetText(JoinedVisibleLogs(true));
        Windows::ApplicationModel::DataTransfer::Clipboard::SetContent(package);
    }



    void MainWindow::CopyAllLogsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        Windows::ApplicationModel::DataTransfer::DataPackage package;
        package.SetText(JoinedVisibleLogs(false));
        Windows::ApplicationModel::DataTransfer::Clipboard::SetContent(package);
    }



    void MainWindow::ExportLogsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        try
        {
            std::filesystem::path const directory = UiStateDirectory() / L"exports";
            std::filesystem::create_directories(directory);
            std::filesystem::path const path = directory / (L"ui-logs-" + TimestampForFilename() + L".log");
            std::ofstream stream(path, std::ios::binary | std::ios::trunc);
            std::string const encoded = WideToUtf8(JoinedVisibleLogs(false));
            stream.write(encoded.data(), static_cast<std::streamsize>(encoded.size()));
            stream.close();
            if (!stream)
            {
                throw std::runtime_error("could not finish writing the log export");
            }
            LogCountText().Text(hstring(L"Exported to " + path.wstring()));
        }
        catch (std::exception const& failure)
        {
            ShowError(L"Log export failed", to_hstring(failure.what()));
        }
    }



    void MainWindow::ExportDiagnosticsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        ExportDiagnosticsAsync();
    }



    fire_and_forget MainWindow::ExportDiagnosticsAsync()
    {
        auto lifetime = get_strong();
        if (commandInFlight_)
        {
            co_return;
        }
        commandInFlight_ = true;
        SetBusy(true);
        auto const queue = DispatcherQueue();
        IJsonValue result{nullptr};
        hstring error;
        co_await resume_background();
        try
        {
            result = gnirehtet::ipc::SendRequest(L"export_diagnostics", JsonObject{});
        }
        catch (std::exception const& failure)
        {
            error = to_hstring(failure.what());
        }
        queue.TryEnqueue([this, lifetime, result, error]() {
            commandInFlight_ = false;
            SetBusy(false);
            if (!error.empty())
            {
                ShowError(L"Diagnostics export failed", error);
                return;
            }
            if (result && result.ValueType() == JsonValueType::Object)
            {
                DiagnosticPath().Text(result.GetObject().GetNamedString(L"path", L"Bundle created"));
            }
        });
    }



    std::filesystem::path MainWindow::UiStateDirectory()
    {
        PWSTR rawPath{};
        check_hresult(SHGetKnownFolderPath(FOLDERID_LocalAppData, KF_FLAG_CREATE, nullptr, &rawPath));
        std::filesystem::path path(rawPath);
        CoTaskMemFree(rawPath);
        return path / L"GnirehtetNg";
    }



    void MainWindow::SaveSettingsButton_Click(IInspectable const&, RoutedEventArgs const&)
    {
        try
        {
            SaveSettings();
            SettingsStatus().Text(L"Settings saved locally. They apply to the next Start or Restart.");
        }
        catch (std::exception const& failure)
        {
            SettingsStatus().Text(to_hstring(std::string("Could not save settings: ") + failure.what()));
        }
    }



    void MainWindow::SaveSettings()
    {
        JsonObject settings;
        WriteSessionSettingsFields(settings);
        settings.SetNamedValue(L"minimize_to_tray", JsonValue::CreateBooleanValue(MinimizeToTrayToggle().IsOn()));



        std::filesystem::path const directory = UiStateDirectory();
        std::filesystem::create_directories(directory);
        std::filesystem::path const path = directory / L"ui-settings.json";
        std::filesystem::path const temporary = directory / L"ui-settings.json.tmp";
        std::ofstream stream(temporary, std::ios::binary | std::ios::trunc);
        std::string const encoded = WideToUtf8(settings.Stringify());
        stream.write(encoded.data(), static_cast<std::streamsize>(encoded.size()));
        stream.close();
        if (!stream)
        {
            throw std::runtime_error("could not finish writing ui-settings.json");
        }
        if (!MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH))
        {
            std::filesystem::remove(temporary);
            throw std::runtime_error("could not replace ui-settings.json");
        }
    }



    void MainWindow::LoadSettings()
    {
        try
        {
            std::filesystem::path const path = UiStateDirectory() / L"ui-settings.json";
            if (!std::filesystem::is_regular_file(path))
            {
                return;
            }
            std::ifstream stream(path, std::ios::binary);
            std::string encoded((std::istreambuf_iterator<char>(stream)), std::istreambuf_iterator<char>());
            JsonObject const settings = JsonObject::Parse(Utf8ToWide(encoded));
            PortNumber().Value(settings.GetNamedNumber(L"port", 31416));
            DnsText().Text(settings.GetNamedString(L"dns_servers", L""));
            RoutesText().Text(settings.GetNamedString(L"routes", L""));
            AutoReconnectToggle().IsOn(settings.GetNamedBoolean(L"auto_reconnect", true));
            MinimizeToTrayToggle().IsOn(settings.GetNamedBoolean(L"minimize_to_tray", false));
            SettingsStatus().Text(L"Saved settings loaded from LocalAppData.");
        }
        catch (std::exception const& failure)
        {
            SettingsStatus().Text(to_hstring(std::string("Saved settings were ignored: ") + failure.what()));
        }
    }
}
