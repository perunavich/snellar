#include <QtCore/QString>
#include <QtGui/QColor>
#include <QtGui/QGuiApplication>
#include <QtGui/QPalette>

extern "C" void snellar_window_bar(const char *background, const char *text)
{
    QPalette palette = QGuiApplication::palette();
    const QColor foreground = QColor::fromString(QString::fromUtf8(text));
    QColor dimmed = foreground;
    dimmed.setAlphaF(0.5f);
    palette.setColor(QPalette::Window, QColor::fromString(QString::fromUtf8(background)));
    palette.setColor(QPalette::WindowText, foreground);
    palette.setColor(QPalette::Disabled, QPalette::WindowText, dimmed);
    QGuiApplication::setPalette(palette);
}
