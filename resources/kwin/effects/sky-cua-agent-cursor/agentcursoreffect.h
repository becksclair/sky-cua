#pragma once

#include "core/region.h"
#include "effect/effect.h"
#include "systemcursoradapter.h"

#include <QPointF>
#include <QString>

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

private:
    void ensureScene();
    void restoreSystemCursor();

    std::unique_ptr<OffscreenQuickScene> m_scene;
    KWinSystemCursorAdapter m_systemCursor;
    QString m_stateJson;
    QPointF m_cursorPoint;
    bool m_hasCursorPoint = false;
    bool m_cursorVisible = true;
};

} // namespace KWin
