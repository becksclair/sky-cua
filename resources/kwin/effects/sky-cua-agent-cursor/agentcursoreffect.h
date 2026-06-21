#pragma once

#include "core/region.h"
#include "effect/quickeffect.h"
#include "systemcursoradapter.h"

#include <QPointF>
#include <QString>
#include <QTimer>

namespace KWin
{

class SkyCuaAgentCursorEffect : public QuickSceneEffect
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "com.skycua.AgentCursor")

public:
    SkyCuaAgentCursorEffect();
    ~SkyCuaAgentCursorEffect() override;

    void prePaintScreen(ScreenPrePaintData &data) override;

public Q_SLOTS:
    bool SetCursorState(const QString &stateJson);
    void Hide();
    void Show();
    QString StateJson() const;
    QString BuildId() const;

protected:
    QVariantMap initialProperties(LogicalOutput *screen) override;

private:
    void updateSceneViews();
    void restoreSystemCursor();

    void armIdleHideTimer();
    void syncStateJsonVisibility();

    KWinSystemCursorAdapter m_systemCursor;
    QString m_stateJson;
    QString m_cursorSource;
    QPointF m_cursorPoint;
    // Failsafe: when the overlay host dies without hiding the cursor, this
    // timer restores the user's cursor on its own.
    QTimer m_idleHideTimer;
    bool m_hasCursorPoint = false;
    // Start hidden: the effect autoloads with the Plasma session, and it must
    // not hide the user's cursor until an overlay host activates it.
    bool m_cursorVisible = false;
};

} // namespace KWin
