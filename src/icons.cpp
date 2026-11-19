#include <QtCore/QBuffer>
#include <QtCore/QByteArray>
#include <QtGui/QImage>
#include <QtGui/QImageReader>
#include <QtQml/QQmlApplicationEngine>
#include <QtQuick/QQuickImageProvider>

class SvgProvider : public QQuickImageProvider
{
public:
    SvgProvider() : QQuickImageProvider(QQuickImageProvider::Image) {}

    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override
    {
        QByteArray svg = QByteArray::fromBase64(id.toLatin1(), QByteArray::Base64UrlEncoding);
        QBuffer buffer(&svg);
        QImageReader reader(&buffer, "svg");
        if (requestedSize.isValid())
            reader.setScaledSize(requestedSize);
        const QImage image = reader.read();
        if (size)
            *size = image.size();
        return image;
    }
};

extern "C" void snellar_svg_provider(void *engine)
{
    static_cast<QQmlApplicationEngine *>(engine)->addImageProvider(QStringLiteral("svg"), new SvgProvider);
}
