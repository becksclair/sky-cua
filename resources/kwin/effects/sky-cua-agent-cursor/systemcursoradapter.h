#pragma once

namespace KWin
{

class KWinSystemCursorAdapter
{
public:
    KWinSystemCursorAdapter() = default;
    ~KWinSystemCursorAdapter();

    KWinSystemCursorAdapter(const KWinSystemCursorAdapter &) = delete;
    KWinSystemCursorAdapter &operator=(const KWinSystemCursorAdapter &) = delete;

    bool supported() const;
    bool hidden() const;
    void setHidden(bool hidden);
    void restore();

private:
    bool m_hidden = false;
};

} // namespace KWin
