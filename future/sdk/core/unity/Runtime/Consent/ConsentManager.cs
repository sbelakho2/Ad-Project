using System;
using Apex.Mediation.Internal;

namespace Apex.Mediation.Consent
{
    internal sealed class ConsentManager
    {
        private readonly ConsentAutoReader _autoReader;
        private ConsentOptions _current = new();
        private bool _autoReadEnabled;

        public ConsentManager(bool enableAutoRead)
        {
            _autoReadEnabled = enableAutoRead;
            _autoReader = new ConsentAutoReader(new PlayerPrefsStore());

            if (_autoReadEnabled)
            {
                Merge(_autoReader.Read());
            }
        }

        public void EnableAutoRead(bool enabled)
        {
            _autoReadEnabled = enabled;
            if (_autoReadEnabled)
            {
                Merge(_autoReader.Read());
            }
        }

        public void SetConsent(ConsentOptions consent)
        {
            if (consent == null)
            {
                throw new ArgumentNullException(nameof(consent));
            }

            _current = consent.Clone();
            RedactedLogger.LogInfo("Consent updated");
        }

        public ConsentOptions GetConsentSnapshot() => _current.Clone();

        public bool CanShowPersonalizedAds()
        {
            if (_current.CoppaApplies == true)
            {
                return false;
            }

            if (IsUsPrivacyOptOut(_current.UsPrivacyString))
            {
                return false;
            }

            if (_current.LimitAdTracking == true)
            {
                return false;
            }

            if (_current.GdprApplies == true)
            {
                return !string.IsNullOrEmpty(_current.TcfString);
            }

            return true;
        }

        private void Merge(ConsentOptions incoming)
        {
            if (incoming == null)
            {
                return;
            }

            if (!string.IsNullOrEmpty(incoming.TcfString))
            {
                _current.TcfString = incoming.TcfString;
            }

            if (!string.IsNullOrEmpty(incoming.GppString))
            {
                _current.GppString = incoming.GppString;
            }

            if (!string.IsNullOrEmpty(incoming.UsPrivacyString))
            {
                _current.UsPrivacyString = incoming.UsPrivacyString;
            }

            if (incoming.GdprApplies.HasValue)
            {
                _current.GdprApplies = incoming.GdprApplies;
            }

            if (incoming.CoppaApplies.HasValue)
            {
                _current.CoppaApplies = incoming.CoppaApplies;
            }

            if (incoming.LimitAdTracking.HasValue)
            {
                _current.LimitAdTracking = incoming.LimitAdTracking;
            }
        }

        private static bool IsUsPrivacyOptOut(string? usp)
        {
            if (string.IsNullOrWhiteSpace(usp) || usp!.Length < 2)
            {
                return false;
            }

            var flag = char.ToUpperInvariant(usp[1]);
            return flag == 'Y';
        }
    }
}
