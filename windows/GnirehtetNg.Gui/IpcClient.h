#pragma once

#include <stdexcept>
#include <string>

namespace gnirehtet::ipc
{
    class IpcError final : public std::runtime_error
    {
    public:
        IpcError(std::string code, std::string message);
        std::string const& code() const noexcept;

    private:
        std::string code_;
    };

    winrt::Windows::Data::Json::IJsonValue SendRequest(
        winrt::hstring const& messageType,
        winrt::Windows::Data::Json::JsonObject const& payload);
}
