#pragma once

#include <limits>
#include <stdexcept>
#include <string>
#include <string_view>

namespace GnirehtetNgEncoding
{
    inline std::string WideToUtf8(std::wstring_view value)
    {
        if (value.empty())
        {
            return {};
        }
        if (value.size() > static_cast<size_t>(std::numeric_limits<int>::max()))
        {
            throw std::runtime_error("text is too large to encode as UTF-8");
        }
        int const sourceSize = static_cast<int>(value.size());
        int const size = WideCharToMultiByte(
            CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), sourceSize, nullptr, 0, nullptr, nullptr);
        if (size == 0)
        {
            throw std::runtime_error("text is not valid UTF-8");
        }
        std::string result(static_cast<size_t>(size), '\0');
        if (WideCharToMultiByte(
                CP_UTF8,
                WC_ERR_INVALID_CHARS,
                value.data(),
                sourceSize,
                result.data(),
                size,
                nullptr,
                nullptr) == 0)
        {
            throw std::runtime_error("could not encode text as UTF-8");
        }
        return result;
    }

    inline std::wstring Utf8ToWide(std::string const& value)
    {
        if (value.empty())
        {
            return {};
        }
        if (value.size() > static_cast<size_t>(std::numeric_limits<int>::max()))
        {
            throw std::runtime_error("UTF-8 text is too large to decode");
        }
        int const sourceSize = static_cast<int>(value.size());
        int const size = MultiByteToWideChar(
            CP_UTF8, MB_ERR_INVALID_CHARS, value.data(), sourceSize, nullptr, 0);
        if (size == 0)
        {
            throw std::runtime_error("text is not valid UTF-8");
        }
        std::wstring result(static_cast<size_t>(size), L'\0');
        if (MultiByteToWideChar(
                CP_UTF8,
                MB_ERR_INVALID_CHARS,
                value.data(),
                sourceSize,
                result.data(),
                size) == 0)
        {
            throw std::runtime_error("could not decode UTF-8 text");
        }
        return result;
    }
}
