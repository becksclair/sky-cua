#pragma once

#include "core/region.h"
#include "effect/effect.h"
#include "systemcursoradapter.h"

#include <QPointF>
#include <QString>
#include <QTimer>

#include <memory>

namespace KWin
{

class OffscreenQuickScene;

class SkyCuaAgentCursorEffect : public Effect
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "com.skycua.AgentCursor")

public:
    SkyCuaAgentCursorEffect();
    ~SkyCuaAgentCursorEffect() override;

    void prePaintScreen(ScreenPrePaintData &data, std::chrono::milliseconds presentTime) override;
    void paintScreen(const RenderTarget &renderTarget,
                     const RenderViewport &viewport,
                     int mask,
                     const Region &deviceRegion,
                     LogicalOutput *screen) override;

public Q_SLOTS:
    bool SetCursorState(const QString &stateJson);
    void Hide();
    void Show();
    QString StateJson() const;
    QString BuildId() const;

private:
    void ensureScene();
    void restoreSystemCursor();

    void armIdleHideTimer();
    void syncStateJsonVisibility();

    std::unique_ptr<OffscreenQuickScene> m_scene;
    KWinSystemCursorAdapter m_systemCursor;
    QString m_stateJson;
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
