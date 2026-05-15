#include "systemcursoradapter.h"

#include "effect/effecthandler.h"

namespace KWin
{

KWinSystemCursorAdapter::~KWinSystemCursorAdapter()
{
    restore();
}

bool KWinSystemCursorAdapter::supported() const
{
    return true;
}

bool KWinSystemCursorAdapter::hidden() const
{
    return m_hidden;
}

void KWinSystemCursorAdapter::setHidden(bool hidden)
{
    if (m_hidden == hidden) {
        return;
    }

    if (hidden) {
        effects->hideCursor();
    } else {
        effects->showCursor();
    }
    m_hidden = hidden;
}

void KWinSystemCursorAdapter::restore()
{
    setHidden(false);
}

} // namespace KWin
