#include "agentcursoreffect.h"

#include "core/output.h"
#include "effect/effecthandler.h"

#include <QDBusConnection>
#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonParseError>
#include <QJsonValue>
#include <QPointF>
#include <QQuickItem>
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
    const QString qmlPath = QStandardPaths::locate(QStandardPaths::GenericDataLocation, QString::fromLatin1(QmlPath));
    const QString cursorPath = QStandardPaths::locate(QStandardPaths::GenericDataLocation, QString::fromLatin1(CursorPath));
    if (qmlPath.isEmpty() || cursorPath.isEmpty()) {
        qWarning() << "sky-cua agent cursor KWin effect resources are missing"
                   << "qml" << qmlPath
                   << "cursor" << cursorPath;
    } else {
        m_cursorSource = QUrl::fromLocalFile(cursorPath).toString();
        setSource(QUrl::fromLocalFile(qmlPath));
    }
    setRunning(false);

    m_idleHideTimer.setSingleShot(true);
    m_idleHideTimer.setInterval(IdleHideTimeoutMs);
    QObject::connect(&m_idleHideTimer, &QTimer::timeout, this, [this] {
        if (!m_cursorVisible) {
            return;
        }
        m_cursorVisible = false;
        syncStateJsonVisibility();
        setRunning(false);
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

void SkyCuaAgentCursorEffect::prePaintScreen(ScreenPrePaintData &data)
{
    if (!m_cursorVisible) {
        restoreSystemCursor();
    } else if (m_systemCursor.supported()) {
        m_systemCursor.setHidden(true);
    }
    QuickSceneEffect::prePaintScreen(data);
    updateSceneViews();
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
    setRunning(m_cursorVisible && !m_cursorSource.isEmpty());
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
    setRunning(false);
    m_idleHideTimer.stop();
    syncStateJsonVisibility();
    restoreSystemCursor();
    effects->addRepaintFull();
}

void SkyCuaAgentCursorEffect::Show()
{
    m_cursorVisible = true;
    setRunning(!m_cursorSource.isEmpty());
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

QVariantMap SkyCuaAgentCursorEffect::initialProperties(LogicalOutput *screen)
{
    Q_UNUSED(screen)
    return {
        {QStringLiteral("cursorSource"), m_cursorSource},
    };
}

void SkyCuaAgentCursorEffect::updateSceneViews()
{
    const QList<LogicalOutput *> screens = effects->screens();
    for (LogicalOutput *screen : screens) {
        QuickSceneView *view = viewForScreen(screen);
        if (!view || !view->rootItem()) {
            continue;
        }
        QQuickItem *root = view->rootItem();
        if (!m_cursorVisible) {
            root->setVisible(false);
            continue;
        }

        const Rect rect = screen->geometry();
        const QPointF cursorPoint = m_hasCursorPoint
            ? m_cursorPoint
            : QPointF(rect.x() + (rect.width() / 2.0), rect.y() + (rect.height() / 2.0));
        if (m_hasCursorPoint
            && (cursorPoint.x() < rect.x() || cursorPoint.x() >= rect.x() + rect.width()
                || cursorPoint.y() < rect.y() || cursorPoint.y() >= rect.y() + rect.height())) {
            root->setVisible(false);
            continue;
        }

        root->setVisible(true);
        root->setWidth(CursorWidth);
        root->setHeight(CursorHeight);
        root->setX(std::round(cursorPoint.x() - rect.x()) - CursorHotspotX);
        root->setY(std::round(cursorPoint.y() - rect.y()) - CursorHotspotY);
        view->setDevicePixelRatio(screen->scale());
        view->scheduleRepaint();
    }
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
