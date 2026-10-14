#if canImport(UIKit) && canImport(WebKit)
import UIKit
import WebKit

/// Lightweight WKWebView wrapper for rich-media creatives with memory-pressure handling.
@available(iOSApplicationExtension, unavailable)
@MainActor
final class RichMediaAdView: UIView {
    private(set) var webView: WKWebView?

    override init(frame: CGRect) {
        super.init(frame: frame)
        setupWebView()
        observeMemoryWarnings()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setupWebView()
        observeMemoryWarnings()
    }

    private func setupWebView() {
        let config = WKWebViewConfiguration()
        config.allowsInlineMediaPlayback = true
        config.allowsAirPlayForMediaPlayback = false

        let view = WKWebView(frame: .zero, configuration: config)
        view.scrollView.isScrollEnabled = false
        view.navigationDelegate = self
        view.translatesAutoresizingMaskIntoConstraints = false

        addSubview(view)
        NSLayoutConstraint.activate([
            view.leadingAnchor.constraint(equalTo: leadingAnchor),
            view.trailingAnchor.constraint(equalTo: trailingAnchor),
            view.topAnchor.constraint(equalTo: topAnchor),
            view.bottomAnchor.constraint(equalTo: bottomAnchor)
        ])
        webView = view
    }

    private func observeMemoryWarnings() {
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleMemoryWarning),
            name: UIApplication.didReceiveMemoryWarningNotification,
            object: nil
        )
    }

    @objc private func handleMemoryWarning() {
        webView?.stopLoading()
        webView?.loadHTMLString("", baseURL: nil)
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
        webView?.navigationDelegate = nil
        webView?.stopLoading()
        webView = nil
    }
}

extension RichMediaAdView: WKNavigationDelegate {
    // Hook for telemetry if needed
}
#endif
