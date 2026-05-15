#include "agentcursoreffect.h"

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

namespace KWin
{
namespace
{

constexpr int CursorWidth = 23;
constexpr int CursorHeight = 24;
constexpr int CursorHotspotX = 10;
constexpr int CursorHotspotY = 11;
constexpr auto QmlPath = "kwin/effects/sky-cua-agent-cursor/qml/main.qml";
constexpr auto CursorPath = "kwin/effects/sky-cua-agent-cursor/assets/cursor-chat.png";
constexpr auto DBusObjectPath = "/com/skycua/AgentCursor";
constexpr auto DBusInterface = "com.skycua.AgentCursor";

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
    return pointFromValue(state.value(QStringLiteral("model_point")));
}

} // namespace

SkyCuaAgentCursorEffect::SkyCuaAgentCursorEffect()
{
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
    effects->addRepaintFull();
    return true;
}

void SkyCuaAgentCursorEffect::Hide()
{
    m_cursorVisible = false;
    restoreSystemCursor();
    effects->addRepaintFull();
}

void SkyCuaAgentCursorEffect::Show()
{
    m_cursorVisible = true;
    effects->addRepaintFull();
}

QString SkyCuaAgentCursorEffect::StateJson() const
{
    return m_stateJson;
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

void SkyCuaAgentCursorEffect::restoreSystemCursor()
{
    m_systemCursor.restore();
}

} // namespace KWin
