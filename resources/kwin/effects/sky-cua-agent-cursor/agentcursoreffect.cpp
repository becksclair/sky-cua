#include "agentcursoreffect.h"

#include "core/output.h"
#include "core/renderviewport.h"
#include "effect/effecthandler.h"
#include "effect/offscreenquickview.h"

#include <QDBusConnection>
#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonParseError>
#include <QJsonValue>
#include <QPointF>
#include <QStandardPaths>
#include <QUrl>
#include <QVariant>

#include <cmath>
#include <optional>

#ifndef SKY_CUA_EFFECT_BUILD_ID
#define SKY_CUA_EFFECT_BUILD_ID "unstamped"
#endif

namespace KWin
{
namespace
{

// The desktop agent cursor renders at 2x the browser/synthetic size (the full
// 46x48 source) for on-screen legibility, with a doubled hotspot.
constexpr int CursorWidth = 46;
constexpr int CursorHeight = 48;
constexpr int CursorHotspotX = 20;
constexpr int CursorHotspotY = 22;
constexpr auto QmlPath = "kwin/effects/sky-cua-agent-cursor/qml/main.qml";
constexpr auto CursorPath = "kwin/effects/sky-cua-agent-cursor/assets/cursor-chat.png";
constexpr auto DBusObjectPath = "/com/skycua/AgentCursor";
constexpr auto DBusInterface = "com.skycua.AgentCursor";
// The agent cursor must never outlive its driver: if the overlay host stops
// refreshing the state (crashed service, killed host, abandoned agent turn),
// hide the overlay and restore the system cursor after this long.
constexpr int IdleHideTimeoutMs = 8000;

std::optional<QPointF> pointFromValue(const QJsonValue &value)
{
    if (!value.isObject()) {
        return std::nullopt;
    }
    const QJsonObject object = value.toObject();
    const QJsonValue xValue = object.value(QStringLiteral("x"));
    const QJsonValue yValue = object.value(QStringLiteral("y"));
    if (!xValue.isDouble() || !yValue.isDouble()) {
        return std::nullopt;
    }
    const double x = xValue.toDouble();
    const double y = yValue.toDouble();
    if (!std::isfinite(x) || !std::isfinite(y)) {
        return std::nullopt;
    }
    return QPointF(x, y);
}

std::optional<QPointF> pointFromState(const QJsonObject &state)
{
    if (auto nativePoint = pointFromValue(state.value(QStringLiteral("native_point")))) {
        return nativePoint;
    }
    // The model point is usually in stream/model pixels; only desktop-logical
    // coordinates can be placed directly in KWin's scene space.
    const QJsonValue modelValue = state.value(QStringLiteral("model_point"));
    if (modelValue.isObject()
        && modelValue.toObject().value(QStringLiteral("coordinate_space")).toString()
            == QStringLiteral("desktop_logical")) {
        return pointFromValue(modelValue);
    }
    return std::nullopt;
}

} // namespace

SkyCuaAgentCursorEffect::SkyCuaAgentCursorEffect()
{
    m_idleHideTimer.setSingleShot(true);
    m_idleHideTimer.setInterval(IdleHideTimeoutMs);
    QObject::connect(&m_idleHideTimer, &QTimer::timeout, this, [this] {
        if (!m_cursorVisible) {
            return;
        }
        m_cursorVisible = false;
        syncStateJsonVisibility();
        restoreSystemCursor();
        effects->addRepaintFull();
    });

    QDBusConnection::sessionBus().registerObject(
        QString::fromLatin1(DBusObjectPath),
        QString::fromLatin1(DBusInterface),
        this,
        QDBusConnection::ExportAllSlots);
}

SkyCuaAgentCursorEffect::~SkyCuaAgentCursorEffect()
{
    QDBusConnection::sessionBus().unregisterObject(QString::fromLatin1(DBusObjectPath));
    restoreSystemCursor();
}

void SkyCuaAgentCursorEffect::prePaintScreen(ScreenPrePaintData &data, std::chrono::milliseconds presentTime)
{
    effects->prePaintScreen(data, presentTime);
    ensureScene();
}

void SkyCuaAgentCursorEffect::paintScreen(const RenderTarget &renderTarget,
                                          const RenderViewport &viewport,
                                          int mask,
                                          const Region &deviceRegion,
                                          LogicalOutput *screen)
{
    effects->paintScreen(renderTarget, viewport, mask, deviceRegion, screen);

    if (!m_scene) {
        restoreSystemCursor();
        return;
    }
    if (!m_cursorVisible) {
        restoreSystemCursor();
        return;
    }

    if (m_systemCursor.supported()) {
        m_systemCursor.setHidden(true);
    }
    const auto rect = viewport.renderRect();
    const QPointF cursorPoint = m_hasCursorPoint
        ? m_cursorPoint
        : QPointF(rect.x() + (rect.width() / 2.0), rect.y() + (rect.height() / 2.0));
    // The agent cursor exists at exactly one desktop-logical location, so draw it
    // only on the output that contains it. paintScreen() runs once per output;
    // renderRect() is this output's global-logical geometry (origin included), so
    // a point outside it belongs to another output and is skipped here. The
    // centered fallback (no cursor point) keeps drawing per output unchanged.
    if (m_hasCursorPoint
        && (cursorPoint.x() < rect.x() || cursorPoint.x() >= rect.x() + rect.width()
            || cursorPoint.y() < rect.y() || cursorPoint.y() >= rect.y() + rect.height())) {
        return;
    }
    // Pin the offscreen scene's devicePixelRatio to this output's compositor scale
    // rather than the QScreen DPR, which KWin quantizes for fractional scales. The
    // viewport already maps global-logical geometry to device pixels through its
    // projection matrix; the DPR only governs the cursor texture's own resolution,
    // so a mismatch on a mixed-scale secondary output skews the rendered position.
    if (screen) {
        m_scene->setDevicePixelRatio(screen->scale());
    }
    const int left = static_cast<int>(std::round(cursorPoint.x())) - CursorHotspotX;
    const int top = static_cast<int>(std::round(cursorPoint.y())) - CursorHotspotY;
    m_scene->setGeometry(QRect(left, top, CursorWidth, CursorHeight));
    effects->renderOffscreenQuickView(renderTarget, viewport, m_scene.get());
}

bool SkyCuaAgentCursorEffect::SetCursorState(const QString &stateJson)
{
    QJsonParseError parseError;
    const QJsonDocument document = QJsonDocument::fromJson(stateJson.toUtf8(), &parseError);
    if (parseError.error != QJsonParseError::NoError || !document.isObject()) {
        qWarning() << "sky-cua agent cursor KWin effect received invalid AgentCursorState JSON"
                   << parseError.errorString();
        return false;
    }

    const QJsonObject state = document.object();
    if (auto point = pointFromState(state)) {
        m_cursorPoint = *point;
        m_hasCursorPoint = true;
    } else {
        m_hasCursorPoint = false;
    }
    m_cursorVisible = state.value(QStringLiteral("visible")).toBool(true);
    m_stateJson = QString::fromUtf8(QJsonDocument(state).toJson(QJsonDocument::Compact));
    if (!m_cursorVisible) {
        restoreSystemCursor();
    }
    armIdleHideTimer();
    effects->addRepaintFull();
    return true;
}

void SkyCuaAgentCursorEffect::Hide()
{
    m_cursorVisible = false;
    m_idleHideTimer.stop();
    syncStateJsonVisibility();
    restoreSystemCursor();
    effects->addRepaintFull();
}

void SkyCuaAgentCursorEffect::Show()
{
    m_cursorVisible = true;
    syncStateJsonVisibility();
    armIdleHideTimer();
    effects->addRepaintFull();
}

QString SkyCuaAgentCursorEffect::StateJson() const
{
    return m_stateJson;
}

QString SkyCuaAgentCursorEffect::BuildId() const
{
    return QStringLiteral(SKY_CUA_EFFECT_BUILD_ID);
}

void SkyCuaAgentCursorEffect::ensureScene()
{
    if (m_scene) {
        return;
    }

    const QString qmlPath = QStandardPaths::locate(QStandardPaths::GenericDataLocation, QString::fromLatin1(QmlPath));
    const QString cursorPath = QStandardPaths::locate(QStandardPaths::GenericDataLocation, QString::fromLatin1(CursorPath));
    if (qmlPath.isEmpty() || cursorPath.isEmpty()) {
        qWarning() << "sky-cua agent cursor KWin effect resources are missing"
                   << "qml" << qmlPath
                   << "cursor" << cursorPath;
        return;
    }

    m_scene = std::make_unique<OffscreenQuickScene>(OffscreenQuickView::ExportMode::Texture, true);
    QObject::connect(m_scene.get(), &OffscreenQuickView::repaintNeeded, [] {
        effects->addRepaintFull();
    });
    m_scene->setSource(
        QUrl::fromLocalFile(qmlPath),
        {
            {QStringLiteral("cursorSource"), QUrl::fromLocalFile(cursorPath).toString()},
        });
    m_scene->show();
}

// Keep the StateJson introspection honest: Hide/Show and the idle failsafe
// change visibility without a new SetCursorState payload.
void SkyCuaAgentCursorEffect::syncStateJsonVisibility()
{
    if (m_stateJson.isEmpty()) {
        return;
    }
    QJsonParseError parseError;
    QJsonDocument document = QJsonDocument::fromJson(m_stateJson.toUtf8(), &parseError);
    if (parseError.error != QJsonParseError::NoError || !document.isObject()) {
        return;
    }
    QJsonObject state = document.object();
    state.insert(QStringLiteral("visible"), m_cursorVisible);
    m_stateJson = QString::fromUtf8(QJsonDocument(state).toJson(QJsonDocument::Compact));
}

void SkyCuaAgentCursorEffect::armIdleHideTimer()
{
    if (m_cursorVisible) {
        m_idleHideTimer.start();
    } else {
        m_idleHideTimer.stop();
    }
}

void SkyCuaAgentCursorEffect::restoreSystemCursor()
{
    m_systemCursor.restore();
}

} // namespace KWin
