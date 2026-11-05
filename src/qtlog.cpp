#include <QtCore/QByteArray>
#include <QtCore/QString>
#include <QtCore/qglobal.h>
#include <QtCore/qlogging.h>
#include <cstdio>

extern "C" void snellar_qt_message(int level, const char *text);

static void forward(QtMsgType type, const QMessageLogContext &context, const QString &message)
{
    const QByteArray line = qFormatLogMessage(type, context, message).toUtf8();
    std::fprintf(stderr, "%s\n", line.constData());
    std::fflush(stderr);
    snellar_qt_message(static_cast<int>(type), line.constData());
}

extern "C" void snellar_install_message_handler()
{
    qInstallMessageHandler(forward);
}

extern "C" const char *snellar_qt_version()
{
    return qVersion();
}
