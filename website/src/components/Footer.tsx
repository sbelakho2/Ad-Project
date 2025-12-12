import SocialIconLink from '@/components/SocialIconLink';

/**
 * Footer with multi-column layout
 * Reference: Design.md § "Footer"
 */
export default function Footer() {
  return (
    <footer className="bg-white text-gray-700 py-16 border-t" style={{borderColor:'var(--gray-200)'}}>
      <div className="container">
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-5 gap-8 mb-12">
          {/* About */}
          <div>
            <h3 className="text-sm font-semibold text-gray-900 mb-4 pb-2 border-b" style={{borderColor:'var(--gray-200)'}}>
              About ApexMediation
            </h3>
            <ul className="space-y-2 text-sm">
              <li><a href="/company" className="text-brand-600 hover:text-brand-700 hover:underline">Company</a></li>
              <li><a href="/contact" className="text-brand-600 hover:text-brand-700 hover:underline">Contact</a></li>
              <li><a href="/security" className="text-brand-600 hover:text-brand-700 hover:underline">Security</a></li>
            </ul>
            <div className="mt-6 text-xs text-gray-500">
              <p>Harju maakond, Tallinn</p>
              <p>Kesklinna linnaosa, Narva mnt 5</p>
              <p>10117, Estonia</p>
            </div>
          </div>

          {/* Product */}
          <div>
            <h3 className="text-sm font-semibold text-gray-900 mb-4 pb-2 border-b" style={{borderColor:'var(--gray-200)'}}>
              Product
            </h3>
            <ul className="space-y-2 text-sm">
              <li><a href="/product" className="text-brand-600 hover:text-brand-700 hover:underline">Product</a></li>
              <li><a href="/pricing" className="text-brand-600 hover:text-brand-700 hover:underline">Pricing</a></li>
              <li><a href="/developers" className="text-brand-600 hover:text-brand-700 hover:underline">Developers</a></li>
            </ul>
          </div>

          {/* Resources */}
          <div>
            <h3 className="text-sm font-semibold text-gray-900 mb-4 pb-2 border-b" style={{borderColor:'var(--gray-200)'}}>
              Resources
            </h3>
            <ul className="space-y-2 text-sm">
              <li><a href="https://docs.apexmediation.ee" className="text-brand-600 hover:text-brand-700 hover:underline">Documentation</a></li>
              <li><a href="https://github.com/sbelakho2/apex-mediation-platform" className="text-brand-600 hover:text-brand-700 hover:underline">GitHub</a></li>
              <li><a href="https://status.apexmediation.ee" className="text-brand-600 hover:text-brand-700 hover:underline">Status</a></li>
            </ul>
          </div>

          {/* Legal */}
          <div>
            <h3 className="text-sm font-semibold text-gray-900 mb-4 pb-2 border-b" style={{borderColor:'var(--gray-200)'}}>
              Legal
            </h3>
            <ul className="space-y-2 text-sm">
              <li><a href="/privacy" className="text-brand-600 hover:text-brand-700 hover:underline">Privacy Policy</a></li>
              <li><a href="/terms" className="text-brand-600 hover:text-brand-700 hover:underline">Terms of Service</a></li>
              <li><a href="/dpa" className="text-brand-600 hover:text-brand-700 hover:underline">Data Processing Addendum</a></li>
              <li><a href="/gdpr" className="text-brand-600 hover:text-brand-700 hover:underline">GDPR</a></li>
              <li><a href="/security" className="text-brand-600 hover:text-brand-700 hover:underline">Security</a></li>
            </ul>
          </div>

          {/* Follow Us */}
          <div>
            <h3 className="text-sm font-semibold text-gray-900 mb-4 pb-2 border-b" style={{borderColor:'var(--gray-200)'}}>
              Follow Us
            </h3>
            <div className="flex gap-4">
              <SocialIconLink href="https://twitter.com/apexmediation" label="Twitter">
                <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z" />
                </svg>
              </SocialIconLink>
              <SocialIconLink href="https://github.com/sbelakho2/apex-mediation-platform" label="GitHub">
                <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
                  <path
                    fillRule="evenodd"
                    d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z"
                    clipRule="evenodd"
                  />
                </svg>
              </SocialIconLink>
              <SocialIconLink href="https://linkedin.com/company/apexmediation" label="LinkedIn">
                <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 5.455v6.286zM5.337 7.433c-1.144 0-2.063-.926-2.063-2.065 0-1.138.92-2.063 2.063-2.063 1.14 0 2.064.925 2.064 2.063 0 1.139-.925 2.065-2.064 2.065zm1.782 13.019H3.555V9h3.564v11.452zM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.222 0h.003z" />
                </svg>
              </SocialIconLink>
            </div>
          </div>
        </div>

        {/* Bottom Section */}
        <div className="border-t pt-8" style={{borderColor:'var(--gray-200)'}}>
          <div className="flex flex-col md:flex-row justify-between items-center gap-4">
            <p className="text-sm text-gray-600">
              © 2025 ApexMediation. All rights reserved.
            </p>
            <div className="flex gap-6 text-sm">
              <a href="/privacy" className="text-brand-600 hover:text-brand-700 transition-colors">Privacy</a>
              <a href="/terms" className="text-brand-600 hover:text-brand-700 transition-colors">Terms</a>
              <a href="/cookies" className="text-brand-600 hover:text-brand-700 transition-colors">Cookies</a>
            </div>
          </div>
        </div>
      </div>
    </footer>
  );
}
