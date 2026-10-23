<script lang="ts">
  import LegalPage from '$lib/components/LegalPage.svelte';
</script>

<LegalPage title="Privacy policy" updated="October 23, 2026">
  <p>
    chessgpt (chessgpt.com, "we") is a free, open-source chess coach. This policy explains what we collect, why, who
    else is involved, and how to delete it. The short version: we collect only what is needed to run the coach, we
    never sell your data or show ads, and you can delete everything at any time.
  </p>

  <h2>Using chessgpt without an account</h2>
  <p>
    As a guest, your games, analyses and drills are stored only in your browser (IndexedDB). When you review games by
    username, your browser fetches them directly from Lichess or Chess.com. Some features (drills, and the coach when it
    runs on your own device) send the position you are working on through our server to be processed; it is not stored.
  </p>

  <h2>What we store when you create an account</h2>
  <ul>
    <li><b>Sign-in details.</b> Your email address and a salted Argon2 hash of your password, or, if you sign in with Lichess or Chess.com, the account id and username that service gives us.</li>
    <li><b>Your chess data.</b> Games you import, engine analyses, coach explanations, drills, puzzles, progress estimates and coach chats.</li>
    <li><b>Settings.</b> Your rating, linked Lichess and Chess.com usernames, and, if you add them, a Lichess API token and API keys for an AI provider. Tokens and keys are encrypted at rest.</li>
    <li><b>Connections.</b> If you connect chessgpt to Claude or ChatGPT, the access tokens for that connection.</li>
    <li><b>A session cookie</b> (<code>chessgpt_session</code>) that keeps you signed in. It is HttpOnly and expires after 30 days.</li>
  </ul>

  <h2>Sign in with Chess.com</h2>
  <p>
    When you choose "Continue with Chess.com", Chess.com asks for your permission and then tells us your Chess.com
    account id and username. That is all we keep. We never see your Chess.com password, we do not store Chess.com
    access tokens, and we do not keep other profile details (such as email, avatar or country) even if Chess.com
    includes them. Your username is used to import your games from Chess.com's public data API and keep them in sync.
    We only read data; chessgpt never posts, plays or changes anything on your Chess.com account. You can unlink
    Chess.com in Settings and revoke chessgpt's access from your Chess.com account settings at any time. Sign in with
    Lichess works the same way; the Lichess token is stored (encrypted) because it also unlocks the opening explorer.
  </p>

  <h2>How we use it</h2>
  <ul>
    <li>To provide the service: import and analyse your games, explain mistakes, build drills and track progress.</li>
    <li>To keep the service secure and working, for example to stop abuse.</li>
  </ul>
  <p>
    We do not sell or rent personal data, do not use it for advertising, and do not use your games, chats or account
    data to train AI models. Our coach models are trained on public game databases.
  </p>

  <h2>Services we rely on</h2>
  <ul>
    <li><b>Cloudflare</b> serves the website and routes traffic to our server.</li>
    <li><b>Oracle Cloud</b> hosts the server and database, in the United States.</li>
    <li><b>Google Analytics</b> counts visits and page views on chessgpt.com. It sets its own cookies; you can block it with any content blocker or Google's opt-out add-on.</li>
    <li><b>Lichess and Chess.com</b> provide sign-in and your public games when you use those features. Their own privacy policies apply to your accounts there.</li>
    <li><b>Hugging Face</b> hosts the coach model your browser downloads if you turn on the on-device coach.</li>
    <li><b>An AI provider you choose.</b> If you add your own API key (for example Anthropic or OpenAI), the positions and questions you send to the coach go to that provider under its terms.</li>
    <li><b>Claude or ChatGPT</b>, only if you connect chessgpt to them; they can then read the chess data you ask them about.</li>
  </ul>

  <h2>Keeping and deleting your data</h2>
  <p>
    We keep your data while your account exists. You can delete single games at any time, and <b>Settings → Delete
    account</b> permanently removes your account and everything stored with it from our database. Unlinking Chess.com
    or Lichess stops syncing. Server logs (IP address, browser and request) are used only for operations and kept for a
    short time. Guest data lives in your browser; clearing site data removes it.
  </p>

  <h2>Security</h2>
  <p>
    All traffic uses HTTPS. Passwords are hashed, tokens and API keys are encrypted, and session and connection tokens
    are stored only as hashes. No system is perfectly secure; if we learn of a breach affecting you, we will tell you.
  </p>

  <h2>Children</h2>
  <p>
    Accounts are for people 13 or older (or the minimum age in your country). We do not knowingly collect personal data
    from younger children; if you believe a child has created an account, contact us and we will delete it.
  </p>

  <h2>Your rights</h2>
  <p>
    Depending on where you live (for example under the GDPR or CCPA) you can ask to access, correct, export or delete
    your personal data, or object to how it is used. Most of this you can do yourself in Settings; for anything else,
    contact us below.
  </p>

  <h2>Changes and contact</h2>
  <p>
    If we change this policy we will update the date above, and tell signed-in users about significant changes. The
    source code that does everything described here is public at
    <a href="https://github.com/andrewnakas/ChessGpt">github.com/andrewnakas/ChessGpt</a>. Questions or requests:
    <a href="https://github.com/andrewnakas/ChessGpt/issues">open an issue</a>.
  </p>
</LegalPage>
