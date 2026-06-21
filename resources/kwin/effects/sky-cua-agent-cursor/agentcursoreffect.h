#pragma once

#include "effect/effect.h"
#include "systemcursoradapter.h"

#include <QString>
#include <QTimer>
#include <QPointF>

#include <chrono>

namespace KWin
{

class SkyCuaAgentCursorEffect : public Effect
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "com.skycua.AgentCursor")

public:
    SkyCuaAgentCursorEffect();
    ~SkyCuaAgentCursorEffect() override;

#ifdef SKY_CUA_KWIN_PREPAINT_HAS_PRESENT_TIME
    void prePaintScreen(ScreenPrePaintData &data, std::chrono::milliseconds presentTime) override;
#else
    void prePaintScreen(ScreenPrePaintData &data) override;
#endif

#ifdef SKY_CUA_KWIN_EFFECT_HAS_POINTER_MOTION
    void pointerMotion(PointerMotionEvent *event) override;
#endif

public Q_SLOTS:
    bool SetCursorState(const QString &stateJson);
    void Hide();
    void Show();
    QString StateJson() const;
    QString PointerStateJson() const;
    QString BuildId() const;

Q_SIGNALS:
    void PointerMoved(double x, double y, qulonglong sequence);

private:
    void applySystemCursorState();
    void restoreSystemCursor();
    void publishPointerPosition(const QPointF &position);

    void armIdleHideTimer();
    void syncStateJsonVisibility();

    KWinSystemCursorAdapter m_systemCursor;
    QString m_stateJson;
    // Failsafe: when the overlay host dies without hiding the cursor, this
    // timer restores the user's cursor on its own.
    QTimer m_idleHideTimer;
    // Start visible=false: autoloading with Plasma must not hide the user's
    // cursor until the layer-shell overlay host explicitly activates the shim.
    bool m_cursorVisible = false;
    bool m_hasLastPointerPosition = false;
    QPointF m_lastPointerPosition;
    qulonglong m_pointerSequence = 0;
};

} // namespace KWin
