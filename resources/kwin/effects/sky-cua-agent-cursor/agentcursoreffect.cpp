#include "agentcursoreffect.h"

#include "effect/effecthandler.h"

#include <QDBusConnection>
#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonParseError>
#include <QPointF>

#include <cmath>

#ifndef SKY_CUA_EFFECT_BUILD_ID
#define SKY_CUA_EFFECT_BUILD_ID "unstamped"
#endif

namespace KWin
{
namespace
{

constexpr auto DBusObjectPath = "/com/skycua/AgentCursor";
constexpr auto DBusInterface = "com.skycua.AgentCursor";
// The shim must never hide the user's compositor cursor forever if the overlay
// host or service dies after showing the layer-shell visual overlay.
constexpr int IdleHideTimeoutMs = 8000;
// ~250 Hz cursor-position poll while the shim is active, so the host always has
// a fresh position to render at high-refresh panels (it consumes at the display
// rate). publishPointerPosition() dedupes sub-pixel deltas, so a stationary
// cursor emits nothing and the timer only produces signals while the pointer
// actually moves.
constexpr int PointerPollIntervalMs = 4;

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
        m_hasLastPointerPosition = false;
        syncStateJsonVisibility();
        restoreSystemCursor();
        updatePointerPolling();
    });

    m_pointerPollTimer.setInterval(PointerPollIntervalMs);
    QObject::connect(&m_pointerPollTimer, &QTimer::timeout, this, [this] {
        publishPointerPosition(effects->cursorPos());
    });

#ifndef SKY_CUA_KWIN_EFFECT_HAS_POINTER_MOTION
    QObject::connect(
        effects,
        &EffectsHandler::mouseChanged,
        this,
        [this](const QPointF &pos,
               const QPointF &,
               Qt::MouseButtons,
               Qt::MouseButtons,
               Qt::KeyboardModifiers,
               Qt::KeyboardModifiers) {
            publishPointerPosition(pos);
        });
#endif

    QDBusConnection::sessionBus().registerObject(
        QString::fromLatin1(DBusObjectPath),
        QString::fromLatin1(DBusInterface),
        this,
        QDBusConnection::ExportAllSlots | QDBusConnection::ExportAllSignals);
}

SkyCuaAgentCursorEffect::~SkyCuaAgentCursorEffect()
{
    QDBusConnection::sessionBus().unregisterObject(QString::fromLatin1(DBusObjectPath));
    restoreSystemCursor();
}

#ifdef SKY_CUA_KWIN_PREPAINT_HAS_PRESENT_TIME
void SkyCuaAgentCursorEffect::prePaintScreen(ScreenPrePaintData &data, std::chrono::milliseconds presentTime)
#else
void SkyCuaAgentCursorEffect::prePaintScreen(ScreenPrePaintData &data)
#endif
{
    if (m_cursorVisible) {
        m_systemCursor.setHidden(true);
    } else {
        restoreSystemCursor();
    }
#ifdef SKY_CUA_KWIN_PREPAINT_HAS_PRESENT_TIME
    Effect::prePaintScreen(data, presentTime);
#else
    Effect::prePaintScreen(data);
#endif
}

#ifdef SKY_CUA_KWIN_EFFECT_HAS_POINTER_MOTION
void SkyCuaAgentCursorEffect::pointerMotion(PointerMotionEvent *event)
{
    publishPointerPosition(effects->cursorPos());
    Effect::pointerMotion(event);
}
#endif

bool SkyCuaAgentCursorEffect::SetCursorState(const QString &stateJson)
{
    QJsonParseError parseError;
    const QJsonDocument document = QJsonDocument::fromJson(stateJson.toUtf8(), &parseError);
    if (parseError.error != QJsonParseError::NoError || !document.isObject()) {
        qWarning() << "sky-cua KWin cursor shim received invalid AgentCursorState JSON"
                   << parseError.errorString();
        return false;
    }

    const QJsonObject state = document.object();
    m_cursorVisible = state.value(QStringLiteral("visible")).toBool(true);
    m_stateJson = QString::fromUtf8(QJsonDocument(state).toJson(QJsonDocument::Compact));
    applySystemCursorState();
    armIdleHideTimer();
    updatePointerPolling();
    publishPointerPosition(effects->cursorPos());
    return true;
}

void SkyCuaAgentCursorEffect::Hide()
{
    m_cursorVisible = false;
    m_hasLastPointerPosition = false;
    syncStateJsonVisibility();
    m_idleHideTimer.stop();
    restoreSystemCursor();
    updatePointerPolling();
}

void SkyCuaAgentCursorEffect::Show()
{
    m_cursorVisible = true;
    syncStateJsonVisibility();
    armIdleHideTimer();
    applySystemCursorState();
    updatePointerPolling();
    publishPointerPosition(effects->cursorPos());
}

QString SkyCuaAgentCursorEffect::StateJson() const
{
    return m_stateJson;
}

QString SkyCuaAgentCursorEffect::PointerStateJson() const
{
    const QPointF position = effects->cursorPos();
    const bool finite = std::isfinite(position.x()) && std::isfinite(position.y());

    QJsonObject state;
    state.insert(QStringLiteral("ok"), finite);
    state.insert(QStringLiteral("visible"), m_cursorVisible);
    if (finite) {
        QJsonObject pointer;
        pointer.insert(QStringLiteral("x"), position.x());
        pointer.insert(QStringLiteral("y"), position.y());
        pointer.insert(QStringLiteral("coordinate_space"), QStringLiteral("desktop_logical"));
        state.insert(QStringLiteral("pointer"), pointer);
    }
    return QString::fromUtf8(QJsonDocument(state).toJson(QJsonDocument::Compact));
}

QString SkyCuaAgentCursorEffect::BuildId() const
{
    return QStringLiteral(SKY_CUA_EFFECT_BUILD_ID);
}

void SkyCuaAgentCursorEffect::syncStateJsonVisibility()
{
    QJsonObject state;
    if (!m_stateJson.isEmpty()) {
        QJsonParseError parseError;
        const QJsonDocument document = QJsonDocument::fromJson(m_stateJson.toUtf8(), &parseError);
        if (parseError.error == QJsonParseError::NoError && document.isObject()) {
            state = document.object();
        }
    }
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

void SkyCuaAgentCursorEffect::updatePointerPolling()
{
    if (m_cursorVisible) {
        if (!m_pointerPollTimer.isActive()) {
            m_pointerPollTimer.start();
        }
    } else {
        m_pointerPollTimer.stop();
    }
}

void SkyCuaAgentCursorEffect::applySystemCursorState()
{
    if (m_cursorVisible) {
        m_systemCursor.setHidden(true);
    } else {
        restoreSystemCursor();
    }
}

void SkyCuaAgentCursorEffect::restoreSystemCursor()
{
    m_systemCursor.restore();
}

void SkyCuaAgentCursorEffect::publishPointerPosition(const QPointF &position)
{
    if (!m_cursorVisible) {
        return;
    }
    if (!std::isfinite(position.x()) || !std::isfinite(position.y())) {
        return;
    }
    if (m_hasLastPointerPosition
        && std::abs(m_lastPointerPosition.x() - position.x()) < 0.25
        && std::abs(m_lastPointerPosition.y() - position.y()) < 0.25) {
        return;
    }
    m_hasLastPointerPosition = true;
    m_lastPointerPosition = position;
    ++m_pointerSequence;
    Q_EMIT PointerMoved(position.x(), position.y(), m_pointerSequence);
}

} // namespace KWin
