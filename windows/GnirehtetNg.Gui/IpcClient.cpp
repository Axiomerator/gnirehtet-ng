#include "pch.h"
#include "IpcClient.h"
#include "Encoding.h"

#include <array>
#include <atomic>
#include <filesystem>
#include <limits>
#include <sstream>
#include <string>

namespace
{
    using namespace winrt::Windows::Data::Json;
    using GnirehtetNgEncoding::Utf8ToWide;
    using GnirehtetNgEncoding::WideToUtf8;

    constexpr wchar_t PipeName[] = LR"(\\.\pipe\gnirehtet-ng-v1)";
    constexpr size_t MaximumMessageBytes = 64 * 1024;
    constexpr wchar_t ExpectedServerImage[] = L"gnirehtet-core.exe";
    std::atomic_uint64_t RequestSequence{1};

    class UniqueHandle
    {
    public:
        explicit UniqueHandle(HANDLE value = INVALID_HANDLE_VALUE) noexcept : value_(value) {}
        ~UniqueHandle()
        {
            if (value_ != INVALID_HANDLE_VALUE && value_ != nullptr)
            {
                CloseHandle(value_);
            }
        }

        UniqueHandle(UniqueHandle const&) = delete;
        UniqueHandle& operator=(UniqueHandle const&) = delete;
        UniqueHandle(UniqueHandle&& other) noexcept : value_(other.value_)
        {
            other.value_ = INVALID_HANDLE_VALUE;
        }
        UniqueHandle& operator=(UniqueHandle&&) = delete;

        HANDLE get() const noexcept { return value_; }

    private:
        HANDLE value_;
    };

    [[noreturn]] void ThrowLastError(char const* operation)
    {
        throw std::runtime_error(
            std::string(operation) + " failed with Win32 error " + std::to_string(GetLastError()));
    }

    std::filesystem::path SiblingExecutable(wchar_t const* filename);

    bool ServerOwnsPipe(HANDLE pipe)
    {
        ULONG serverPid = 0;
        if (!GetNamedPipeServerProcessId(pipe, &serverPid) || serverPid == 0)
        {
            return false;
        }
        UniqueHandle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, serverPid));
        if (process.get() == nullptr)
        {
            return false;
        }
        std::wstring image(32'768, L'\0');
        DWORD size = static_cast<DWORD>(image.size());
        if (!QueryFullProcessImageNameW(process.get(), 0, image.data(), &size))
        {
            return false;
        }
        image.resize(size);
        std::error_code error;
        std::filesystem::path const actual =
            std::filesystem::canonical(std::filesystem::path(image), error);
        if (error)
        {
            return false;
        }
        std::filesystem::path const expected =
            std::filesystem::canonical(SiblingExecutable(ExpectedServerImage), error);
        if (error)
        {
            return false;
        }
        return CompareStringOrdinal(
                   actual.native().c_str(),
                   -1,
                   expected.native().c_str(),
                   -1,
                   TRUE) == CSTR_EQUAL;
    }

    std::filesystem::path SiblingExecutable(wchar_t const* filename)
    {
        std::wstring module(32'768, L'\0');
        DWORD const length = GetModuleFileNameW(nullptr, module.data(), static_cast<DWORD>(module.size()));
        if (length == 0 || length == module.size())
        {
            ThrowLastError("GetModuleFileNameW");
        }
        module.resize(length);
        return std::filesystem::path(module).parent_path() / filename;
    }

    void PrepareSelfContainedEnvironment()
    {
        // Make double-clicking the GUI fully self-contained: when the environment
        // variables are not already set, point them at the files shipped beside the
        // executable so the controller finds ADB, the APK, and the relay without any
        // launcher script or machine configuration.
        std::filesystem::path const directory = SiblingExecutable(L"gnirehtet-core.exe").parent_path();
        struct Variable
        {
            wchar_t const* name;
            std::filesystem::path relative;
        };
        Variable const variables[] = {
            {L"ADB", L"adb\\adb.exe"},
            {L"GNIREHTET_APK", L"gnirehtet-ng.apk"},
            {L"GNIREHTET_NG_RELAY", L"gnirehtet-ng.exe"},
        };
        for (Variable const& variable : variables)
        {
            wchar_t buffer[32'768]{};
            DWORD const existing = GetEnvironmentVariableW(variable.name, buffer, static_cast<DWORD>(std::size(buffer)));
            if (existing != 0)
            {
                continue; // an explicit environment takes precedence
            }
            std::filesystem::path const candidate = directory / variable.relative;
            if (std::filesystem::is_regular_file(candidate))
            {
                SetEnvironmentVariableW(variable.name, candidate.native().c_str());
            }
        }
    }

    void StartCore()
    {
        std::filesystem::path const core = SiblingExecutable(L"gnirehtet-core.exe");
        if (!std::filesystem::is_regular_file(core))
        {
            throw std::runtime_error("gnirehtet-core.exe is missing beside the GUI executable");
        }

        PrepareSelfContainedEnvironment();
        std::wstring application = core.native();
        // the controller monitors this parent PID and cleans up (session, phone app, adb
        // server) when the GUI exits, however it exits (graceful, crash, or force-kill)
        std::wstring commandLine =
            L"\"" + application + L"\" --parent-pid " + std::to_wstring(GetCurrentProcessId());
        STARTUPINFOW startup{};
        startup.cb = sizeof(startup);
        PROCESS_INFORMATION process{};
        if (!CreateProcessW(
                application.c_str(),
                commandLine.data(),
                nullptr,
                nullptr,
                FALSE,
                CREATE_NO_WINDOW | DETACHED_PROCESS,
                nullptr,
                nullptr,
                &startup,
                &process))
        {
            ThrowLastError("CreateProcessW");
        }
        UniqueHandle threadHandle(process.hThread);
        UniqueHandle processHandle(process.hProcess);
    }

    UniqueHandle OpenPipe()
    {
        bool coreStarted = false;
        for (unsigned attempt = 0; attempt < 30; ++attempt)
        {
            if (WaitNamedPipeW(PipeName, 100))
            {
                HANDLE const handle = CreateFileW(
                    PipeName,
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    nullptr,
                    OPEN_EXISTING,
                    SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    nullptr);
                if (handle != INVALID_HANDLE_VALUE)
                {
                    UniqueHandle pipe(handle);
                    if (!ServerOwnsPipe(pipe.get()))
                    {
                        throw std::runtime_error(
                            "The controller pipe server did not verify as gnirehtet-core.exe");
                    }
                    return pipe;
                }
                if (GetLastError() != ERROR_PIPE_BUSY)
                {
                    ThrowLastError("CreateFileW");
                }
            }
            else
            {
                DWORD const error = GetLastError();
                if (!coreStarted && (error == ERROR_FILE_NOT_FOUND || error == ERROR_SEM_TIMEOUT))
                {
                    StartCore();
                    coreStarted = true;
                }
                else if (error != ERROR_FILE_NOT_FOUND && error != ERROR_SEM_TIMEOUT &&
                         error != ERROR_PIPE_BUSY)
                {
                    ThrowLastError("WaitNamedPipeW");
                }
            }
            Sleep(100);
        }
        throw std::runtime_error("controller did not open its named pipe within 3 seconds");
    }

    JsonObject BuildEnvelope(winrt::hstring const& messageType, JsonObject const& payload)
    {
        JsonObject envelope;
        envelope.SetNamedValue(L"version", JsonValue::CreateNumberValue(1));
        std::wstringstream requestId;
        requestId << L"winui-" << GetCurrentProcessId() << L'-' << RequestSequence.fetch_add(1);
        envelope.SetNamedValue(L"id", JsonValue::CreateStringValue(requestId.str()));
        envelope.SetNamedValue(L"type", JsonValue::CreateStringValue(messageType));
        envelope.SetNamedValue(L"payload", payload);
        return envelope;
    }

    std::string Exchange(JsonObject const& envelope)
    {
        std::string request = WideToUtf8(envelope.Stringify()) + '\n';
        if (request.size() > MaximumMessageBytes)
        {
            throw std::runtime_error("IPC request exceeds 64 KiB");
        }
        UniqueHandle pipe = OpenPipe();
        DWORD bytesWritten = 0;
        if (!WriteFile(
                pipe.get(),
                request.data(),
                static_cast<DWORD>(request.size()),
                &bytesWritten,
                nullptr) ||
            bytesWritten != request.size())
        {
            ThrowLastError("WriteFile");
        }

        std::string response;
        std::array<char, 4096> buffer{};
        for (;;)
        {
            DWORD bytesRead = 0;
            BOOL const read = ReadFile(
                pipe.get(), buffer.data(), static_cast<DWORD>(buffer.size()), &bytesRead, nullptr);
            DWORD const error = read ? ERROR_SUCCESS : GetLastError();
            if (!read && error != ERROR_MORE_DATA)
            {
                ThrowLastError("ReadFile");
            }
            response.append(buffer.data(), bytesRead);
            if (!response.empty() && response.back() == '\n')
            {
                break;
            }
            if (response.size() > MaximumMessageBytes)
            {
                throw std::runtime_error("IPC response exceeds 64 KiB");
            }
        }
        return response;
    }
}

namespace gnirehtet::ipc
{
    IpcError::IpcError(std::string code, std::string message)
        : std::runtime_error(std::move(message)), code_(std::move(code))
    {
    }

    std::string const& IpcError::code() const noexcept
    {
        return code_;
    }

    IJsonValue SendRequest(winrt::hstring const& messageType, JsonObject const& payload)
    {
        // The controller serves one request per pipe connection and reconnects between
        // requests, so a transport read can race the disconnect. Retry transport-level
        // failures for read-only requests only: re-issuing start/stop/restart after the
        // request was already executed would duplicate side effects.
        constexpr unsigned MaxTransportAttempts = 4;
        bool const readOnly = messageType == L"get_status" || messageType == L"get_devices" ||
            messageType == L"get_logs" || messageType == L"get_metrics";
        unsigned const attempts = readOnly ? MaxTransportAttempts : 1;
        JsonObject const requestEnvelope = BuildEnvelope(messageType, payload);
        std::string response;
        for (unsigned attempt = 0;; ++attempt)
        {
            try
            {
                response = Exchange(requestEnvelope);
                break;
            }
            catch (std::runtime_error const&)
            {
                if (attempt + 1 >= attempts)
                {
                    throw;
                }
                Sleep(120);
            }
        }
        JsonObject const envelope = JsonObject::Parse(Utf8ToWide(response));
        if (!envelope.GetNamedBoolean(L"ok", false))
        {
            JsonObject const fault = envelope.GetNamedObject(L"payload", JsonObject{});
            std::string const code = WideToUtf8(fault.GetNamedString(L"code", L"ipc_error"));
            std::string const message = WideToUtf8(
                fault.GetNamedString(L"message", L"Controller returned an unspecified error"));
            throw IpcError(code, message);
        }
        return envelope.GetNamedValue(L"payload");
    }
}
